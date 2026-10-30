//! The AVX-512 functions of `core::arch` (and AVX-VNNI's) whose instructions are LLVM
//! intrinsics: compares, reciprocal estimates, scaling, exponents, conversions in every rounding,
//! truncations, compressions and expansions, conflicts, dot products, bit shuffles and
//! intersections. Each is called on floats at the edges (NaNs with payloads, infinities, signed
//! zeros, denormals, ties, values past the integers' range) and integers at the edges of each
//! lane width, under masks of none, all, alternate and single lanes, with every rounding and SAE
//! immediate; what each returns or stores is hashed. Generated from `core::arch`'s sources.
#![allow(clippy::all)]
use core::arch::x86_64::*;
use std::hint::black_box;

const F32: [[u8; 64]; 3] = [
    floats([f32::NAN, -0.0, 0.0, f32::INFINITY, f32::NEG_INFINITY, 1.0e-45, -1.1754942e-38, 1.17549435e-38,
            1.0, -1.5, 2.5, -2.5, 2147483520.0, 2147483648.0, 4294967296.0, -2147483904.0]),
    floats([0.5, -0.5, 1.5, 3.0e38, -3.0e38, 1.0e-40, 7.0, -8.0, f32::from_bits(0x7fa0_0001),
            f32::from_bits(0xffc0_1234), 4294967040.0, -1.0, 0.1, 100.0, 16.0, -0.0]),
    floats([3.25, -7.75, 1.0e10, -1.0e-10, 65504.0, 65520.0, 6.0e-8, 2.0, 0.75, -3.5, 8388609.0,
            33554436.0, 1.0e-38, -9.5, 255.5, f32::NAN]),
];
const F64: [[u8; 64]; 3] = [
    doubles([f64::NAN, -0.0, f64::INFINITY, f64::NEG_INFINITY, 5.0e-324, -2.2250738585072009e-308,
             2.5, -2147483648.5]),
    doubles([0.5, -1.5, 9.3e18, -9.3e18, 1.8446744073709552e19, 4294967295.5, f64::from_bits(0x7ff4_0000_0000_0001),
             1.0e300]),
    doubles([3.0e38, 3.5e38, 1.0e-39, -0.75, 2147483647.5, -3.25, 1.0e-320, f64::NAN]),
];
const INT: [[u8; 64]; 3] = [
    ints([1 << 63, i64::MAX as u64, u64::MAX, 0, 0x7fff_ffff_8000_0000, 0x0000_7fff_ffff_8000, 0x80ff_7f00_0180_ff7f, 0x1234_5678_9abc_def0]),
    ints([0x00ff_0100_ff7f_0080, -0x8000_0001i64 as u64, 0x0101_0101_0101_0101, 0x7f80_807f_fe02_00ff, 1, -2i64 as u64, 0x0000_ffff_0001_0000, 0x4000_0000_0000_0000]),
    ints([0x0f0f_f0f0_3c3c_c3c3, 0x0003_0002_0001_0000, 0x0807_0605_0403_0201, -0x1234_5678i64 as u64, 0x8000_0000_0000_0001, 255, 65535, 0x7fff_7fff_7fff_7fff]),
];
const MASKS: [u64; 7] = [0, u64::MAX, 0x5555_5555_5555_5555, 0xaaaa_aaaa_aaaa_aaaa, 1, 1 << 63 | 1 << 31 | 1 << 15 | 1 << 7, 0x0f0f_00ff_f0f0_ff00];
static SRC: [u8; 128] = {
    let mut bytes = [0u8; 128];
    let mut i = 0;
    while i < 128 {
        bytes[i] = (i as u8).wrapping_mul(73) ^ 0xa5;
        i += 1;
    }
    bytes
};

const fn floats(a: [f32; 16]) -> [u8; 64] { unsafe { core::mem::transmute(a) } }
const fn doubles(a: [f64; 8]) -> [u8; 64] { unsafe { core::mem::transmute(a) } }
const fn ints(a: [u64; 8]) -> [u8; 64] { unsafe { core::mem::transmute(a) } }

/// The first bytes of a pool as a vector, through `black_box` so that no compiler folds the call.
fn ld<T: Copy>(bytes: &[u8; 64]) -> T {
    assert!(size_of::<T>() <= 64);
    unsafe { core::ptr::read_unaligned(black_box(bytes).as_ptr().cast()) }
}

/// FNV-1a of the bytes of `value` into `hash`.
fn mix<T: Copy>(hash: &mut u64, value: &T) {
    let bytes = unsafe { core::slice::from_raw_parts((value as *const T).cast::<u8>(), size_of::<T>()) };
    for &byte in bytes {
        *hash = (*hash ^ u64::from(byte)).wrapping_mul(0x100_0000_01b3);
    }
}

#[target_feature(enable = "avx512f")]
unsafe fn mm512_max_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_max_ps(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_max_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_max_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_max_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_max_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_max_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_max_pd(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_max_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_max_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_max_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_max_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_min_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_min_ps(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_min_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_min_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_min_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_min_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_min_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_min_pd(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_min_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_min_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_min_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_min_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_rcp14_ps(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_rcp14_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_rcp14_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_rcp14_ps(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_rcp14_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_rcp14_ps(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rcp14_ps(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rcp14_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_rcp14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rcp14_ps(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_rcp14_pd(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_rcp14_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_rcp14_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_rcp14_pd(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_rcp14_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_rcp14_pd(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rcp14_pd(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rcp14_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_rcp14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rcp14_pd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_rsqrt14_ps(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_rsqrt14_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_rsqrt14_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_rsqrt14_ps(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_rsqrt14_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_rsqrt14_ps(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rsqrt14_ps(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rsqrt14_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_rsqrt14_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rsqrt14_ps(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_rsqrt14_pd(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_rsqrt14_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_rsqrt14_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_rsqrt14_pd(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_rsqrt14_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_rsqrt14_pd(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rsqrt14_pd(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rsqrt14_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_rsqrt14_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rsqrt14_pd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_getexp_ps(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_getexp_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_getexp_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_getexp_ps(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_getexp_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_getexp_ps(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_getexp_ps(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_getexp_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_ps(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_getexp_pd(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_getexp_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_getexp_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_getexp_pd(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_getexp_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_getexp_pd(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_getexp_pd(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_getexp_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_pd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_ps(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_scalef_ps(ld::<__m256>(&F32[(i + 0) % 3]), ld::<__m256>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_scalef_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3]), ld::<__m256>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_scalef_ps(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3]), ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_scalef_ps(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_scalef_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_ps(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_pd(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_scalef_pd(ld::<__m256d>(&F64[(i + 0) % 3]), ld::<__m256d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_scalef_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3]), ld::<__m256d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_scalef_pd(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3]), ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_scalef_pd(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_scalef_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_pd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_max_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_max_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_max_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_max_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_max_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_max_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_max_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_max_round_ps::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_max_round_ps::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_max_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_max_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_max_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_max_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_max_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_max_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_max_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_max_round_pd::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_max_round_pd::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_min_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_min_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_min_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_min_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_min_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_min_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_min_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_min_round_ps::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_min_round_ps::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_min_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_min_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_min_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_min_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_min_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_min_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_min_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_min_round_pd::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_min_round_pd::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_getexp_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_getexp_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_getexp_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_getexp_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_getexp_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_getexp_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_getexp_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_getexp_round_ps::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_getexp_round_ps::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_getexp_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_getexp_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_getexp_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_getexp_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_getexp_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_getexp_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_getexp_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_getexp_round_pd::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_getexp_round_pd::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_scalef_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_ps::<9>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_ps::<10>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_ps::<11>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_scalef_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_ps::<9>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_ps::<10>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_ps::<11>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_scalef_round_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_ps::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_ps::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_ps::<9>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_ps::<10>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_ps::<11>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_scalef_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_pd::<9>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_pd::<10>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_scalef_round_pd::<11>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_scalef_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_pd::<9>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_pd::<10>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_round_pd::<11>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_scalef_round_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_pd::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_pd::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_pd::<9>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_pd::<10>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_scalef_round_pd::<11>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtps_epi32(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtps_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtps_epi32(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtps_epu32(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtps_epu32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtps_epu32(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtps_epu32(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtps_epu32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtps_epu32(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtps_epu32(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtps_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtps_epu32(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtps_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtps_pd(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtps_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtps_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtps_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtps_pd(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtpslo_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtpslo_pd(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtpslo_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpslo_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtpd_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtpd_ps(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtpd_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpd_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtpd_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtpd_ps(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtpd_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtpd_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtpd_epi32(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtpd_epi32(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtpd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtpd_epu32(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpd_epu32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtpd_epu32(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtpd_epu32(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtpd_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtpd_epu32(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtpd_epu32(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtpd_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtpd_epu32(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtpd_pslo() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtpd_pslo(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtpd_pslo() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpd_pslo(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepi32_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepi32_epi8(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtepi32_epi8(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepi32_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepi64_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepi64_epi16(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtepi64_epi16(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepi64_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepi64_epi8(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtepi64_epi8(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepi64_epi8(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtepi64_epi8(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepi64_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtsepi32_epi16(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtsepi32_epi16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtsepi32_epi16(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtsepi32_epi16(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtsepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtsepi32_epi16(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtsepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtsepi32_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtsepi32_epi8(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtsepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtsepi32_epi8(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtsepi32_epi8(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtsepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtsepi32_epi8(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtsepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtsepi32_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtsepi64_epi32(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtsepi64_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtsepi64_epi32(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtsepi64_epi32(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtsepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtsepi64_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtsepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtsepi64_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtsepi64_epi16(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtsepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtsepi64_epi16(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtsepi64_epi16(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtsepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtsepi64_epi16(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtsepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtsepi64_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtsepi64_epi8(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtsepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtsepi64_epi8(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtsepi64_epi8(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtsepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtsepi64_epi8(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtsepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtsepi64_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtusepi32_epi16(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtusepi32_epi16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtusepi32_epi16(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtusepi32_epi16(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtusepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtusepi32_epi16(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtusepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtusepi32_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtusepi32_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtusepi32_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtusepi32_epi8(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtusepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtusepi32_epi8(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtusepi32_epi8(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtusepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtusepi32_epi8(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtusepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtusepi32_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtusepi32_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtusepi32_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtusepi64_epi32(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtusepi64_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtusepi64_epi32(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtusepi64_epi32(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtusepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtusepi64_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtusepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtusepi64_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtusepi64_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtusepi64_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtusepi64_epi16(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtusepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtusepi64_epi16(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtusepi64_epi16(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtusepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtusepi64_epi16(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtusepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtusepi64_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtusepi64_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtusepi64_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtusepi64_epi8(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtusepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtusepi64_epi8(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtusepi64_epi8(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtusepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtusepi64_epi8(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtusepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtusepi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvtusepi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtusepi64_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvt_roundps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi32::<4>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi32::<8>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi32::<9>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi32::<10>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi32::<11>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvt_roundps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi32::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi32::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi32::<9>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi32::<10>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi32::<11>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvt_roundps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi32::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi32::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi32::<9>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi32::<10>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi32::<11>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvt_roundps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu32::<4>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu32::<8>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu32::<9>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu32::<10>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu32::<11>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvt_roundps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu32::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu32::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu32::<9>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu32::<10>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu32::<11>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvt_roundps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu32::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu32::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu32::<9>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu32::<10>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu32::<11>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvt_roundps_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_pd::<4>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_pd::<8>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvt_roundps_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvt_roundps_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_pd::<4>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_pd::<8>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvt_roundpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi32::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi32::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi32::<9>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi32::<10>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi32::<11>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvt_roundpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi32::<4>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi32::<8>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi32::<9>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi32::<10>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi32::<11>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvt_roundpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi32::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi32::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi32::<9>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi32::<10>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi32::<11>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvt_roundpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu32::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu32::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu32::<9>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu32::<10>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu32::<11>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvt_roundpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu32::<4>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu32::<8>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu32::<9>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu32::<10>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu32::<11>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvt_roundpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu32::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu32::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu32::<9>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu32::<10>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu32::<11>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvt_roundpd_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_ps::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_ps::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_ps::<9>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_ps::<10>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_ps::<11>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvt_roundpd_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_ps::<4>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_ps::<8>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_ps::<9>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_ps::<10>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_ps::<11>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvt_roundpd_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_ps::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_ps::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_ps::<9>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_ps::<10>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_ps::<11>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvt_roundph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundph_ps::<4>(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundph_ps::<8>(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvt_roundph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundph_ps::<4>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundph_ps::<8>(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvt_roundph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundph_ps::<4>(m as __mmask16, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundph_ps::<8>(m as __mmask16, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtph_ps(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtph_ps(m as __mmask16, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtt_roundps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epi32::<4>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epi32::<8>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtt_roundps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epi32::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epi32::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtt_roundps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epi32::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epi32::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtt_roundps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epu32::<4>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epu32::<8>(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtt_roundps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epu32::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epu32::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtt_roundps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epu32::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epu32::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtt_roundpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epi32::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epi32::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtt_roundpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epi32::<4>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epi32::<8>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtt_roundpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epi32::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epi32::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvtt_roundpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epu32::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epu32::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtt_roundpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epu32::<4>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epu32::<8>(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvttps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttps_epi32(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvttps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttps_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvttps_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttps_epi32(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttps_epu32(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttps_epu32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttps_epu32(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvttps_epu32(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttps_epu32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvttps_epu32(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttps_epu32(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttps_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvttps_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvttps_epu32(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvtt_roundpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epu32::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epu32::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvttpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttpd_epi32(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvttpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttpd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvttpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttpd_epi32(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvttpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttpd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvttpd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvttpd_epi32(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttpd_epu32(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttpd_epu32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttpd_epu32(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvttpd_epu32(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttpd_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvttpd_epu32(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttpd_epu32(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttpd_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_cvttpd_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvttpd_epu32(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compress_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_compress_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_compress_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_compress_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compress_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_compress_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_compress_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_compress_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compress_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_compress_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_compress_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_compress_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compress_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_compress_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_compress_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_compress_epi64(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compress_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_compress_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_compress_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_compress_epi64(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compress_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_compress_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_compress_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_compress_epi64(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compress_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_compress_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_compress_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_compress_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compress_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_compress_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_compress_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_compress_ps(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compress_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_compress_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_compress_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_compress_ps(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compress_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_compress_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_compress_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_compress_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compress_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_compress_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_compress_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_compress_pd(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compress_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_compress_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_compress_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_compress_pd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compressstoreu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_compressstoreu_epi32(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compressstoreu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_compressstoreu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compressstoreu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_compressstoreu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compressstoreu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_compressstoreu_epi64(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compressstoreu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_compressstoreu_epi64(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compressstoreu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_compressstoreu_epi64(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compressstoreu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_compressstoreu_ps(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compressstoreu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_compressstoreu_ps(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compressstoreu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_compressstoreu_ps(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_compressstoreu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_compressstoreu_pd(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_compressstoreu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_compressstoreu_pd(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_compressstoreu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_compressstoreu_pd(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expand_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expand_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expand_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expand_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expand_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expand_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expand_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expand_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expand_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expand_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expand_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expand_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expand_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expand_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expand_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expand_epi64(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expand_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expand_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expand_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expand_epi64(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expand_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expand_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expand_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expand_epi64(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expand_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expand_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expand_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expand_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expand_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expand_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expand_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expand_ps(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expand_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expand_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expand_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expand_ps(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expand_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expand_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expand_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expand_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expand_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expand_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expand_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expand_pd(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expand_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expand_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expand_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expand_pd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_sll_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sll_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_sll_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_sll_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_sll_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_sll_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_srl_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_srl_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_srl_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_srl_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_srl_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_srl_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_sll_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sll_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_sll_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_sll_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_sll_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_sll_epi64(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_srl_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_srl_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_srl_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_srl_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_srl_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_srl_epi64(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_sra_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sra_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_sra_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_sra_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_sra_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_sra_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sra_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_sra_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_sra_epi64(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_sra_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_sra_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_sra_epi64(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_sra_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sra_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_sra_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sra_epi64(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_permutevar_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_permutevar_ps(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_permutevar_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_permutevar_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_permutevar_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_permutevar_ps(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_permutevar_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_permutevar_pd(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_permutevar_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_permutevar_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_permutevar_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_permutevar_pd(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cmp_ps_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<0>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<1>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<2>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<3>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<5>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<6>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<7>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<11>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<13>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<14>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<15>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<16>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<17>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<21>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<24>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<27>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_ps_mask::<31>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cmp_ps_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<0>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<1>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<2>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<3>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<5>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<6>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<7>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<11>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<13>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<14>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<15>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<16>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<17>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<21>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<24>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<27>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_ps_mask::<31>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cmp_round_ps_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<0, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<0, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<1, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<1, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<2, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<2, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<3, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<3, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<4, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<4, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<5, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<5, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<6, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<6, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<7, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<7, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<8, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<8, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<11, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<11, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<13, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<13, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<14, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<14, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<15, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<15, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<16, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<16, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<17, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<17, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<21, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<21, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<24, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<24, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<27, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<27, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<31, 4>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_ps_mask::<31, 8>(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cmp_round_ps_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<0, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<0, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<1, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<1, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<2, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<2, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<3, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<3, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<4, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<4, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<5, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<5, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<6, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<6, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<7, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<7, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<8, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<8, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<11, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<11, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<13, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<13, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<14, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<14, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<15, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<15, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<16, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<16, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<17, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<17, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<21, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<21, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<24, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<24, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<27, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<27, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<31, 4>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_ps_mask::<31, 8>(m as __mmask16, ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cmp_pd_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<0>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<1>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<2>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<3>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<5>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<6>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<7>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<11>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<13>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<14>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<15>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<16>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<17>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<21>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<24>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<27>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_pd_mask::<31>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cmp_pd_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<0>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<1>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<2>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<3>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<5>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<6>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<7>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<11>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<13>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<14>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<15>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<16>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<17>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<21>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<24>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<27>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_pd_mask::<31>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_cmp_round_pd_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<0, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<0, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<1, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<1, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<2, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<2, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<3, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<3, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<4, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<4, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<5, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<5, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<6, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<6, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<7, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<7, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<8, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<8, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<11, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<11, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<13, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<13, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<14, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<14, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<15, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<15, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<16, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<16, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<17, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<17, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<21, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<21, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<24, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<24, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<27, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<27, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<31, 4>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cmp_round_pd_mask::<31, 8>(ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cmp_round_pd_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<0, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<0, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<1, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<1, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<2, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<2, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<3, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<3, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<4, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<4, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<5, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<5, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<6, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<6, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<7, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<7, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<8, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<8, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<11, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<11, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<13, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<13, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<14, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<14, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<15, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<15, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<16, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<16, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<17, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<17, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<21, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<21, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<24, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<24, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<27, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<27, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<31, 4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmp_round_pd_mask::<31, 8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3]), ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtsepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtsepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtsepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtusepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtusepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi32_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtusepi32_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtsepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtsepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtsepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtusepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtusepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi32_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtusepi32_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtsepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtsepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtsepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtusepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtusepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi64_storeu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtusepi64_storeu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtsepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtsepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtsepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtusepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtusepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi64_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtusepi64_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtsepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtsepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtsepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtsepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtsepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtsepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_cvtusepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtusepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_cvtusepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_cvtusepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_cvtusepi64_storeu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_cvtusepi64_storeu_epi32(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expandloadu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expandloadu_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expandloadu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expandloadu_epi32(m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expandloadu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expandloadu_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expandloadu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expandloadu_epi32(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expandloadu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expandloadu_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expandloadu_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expandloadu_epi32(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expandloadu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expandloadu_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expandloadu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expandloadu_epi64(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expandloadu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expandloadu_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expandloadu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expandloadu_epi64(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expandloadu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expandloadu_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expandloadu_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expandloadu_epi64(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expandloadu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expandloadu_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expandloadu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expandloadu_ps(m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expandloadu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expandloadu_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expandloadu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expandloadu_ps(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expandloadu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expandloadu_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expandloadu_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expandloadu_ps(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_mask_expandloadu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expandloadu_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm512_maskz_expandloadu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expandloadu_pd(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_mask_expandloadu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expandloadu_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm256_maskz_expandloadu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expandloadu_pd(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_mask_expandloadu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expandloadu_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f,avx512vl")]
unsafe fn mm_maskz_expandloadu_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expandloadu_pd(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_max_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_max_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_max_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_max_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_max_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_max_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_max_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_max_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_min_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_min_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_min_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_min_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_min_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_min_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_min_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_min_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_sqrt_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_sqrt_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_sqrt_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_sqrt_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_rsqrt14_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rsqrt14_ss(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_rsqrt14_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rsqrt14_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_rsqrt14_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rsqrt14_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_rsqrt14_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rsqrt14_sd(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_rsqrt14_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rsqrt14_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_rsqrt14_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rsqrt14_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_rcp14_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rcp14_ss(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_rcp14_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rcp14_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_rcp14_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rcp14_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_rcp14_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_rcp14_sd(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_rcp14_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rcp14_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_rcp14_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_rcp14_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_getexp_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_getexp_ss(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_getexp_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_getexp_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_getexp_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_getexp_sd(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_getexp_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_getexp_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_scalef_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_scalef_ss(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_scalef_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_scalef_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_scalef_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_scalef_sd(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_scalef_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_scalef_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_max_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_max_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_max_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_max_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_max_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_max_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_max_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_max_round_ss::<4>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_max_round_ss::<8>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_max_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_max_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_max_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_max_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_max_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_max_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_max_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_max_round_sd::<4>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_max_round_sd::<8>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_min_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_min_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_min_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_min_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_min_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_min_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_min_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_min_round_ss::<4>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_min_round_ss::<8>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_min_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_min_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_min_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_min_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_min_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_min_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_min_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_min_round_sd::<4>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_min_round_sd::<8>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_sqrt_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_ss::<9>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_ss::<10>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_ss::<11>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_sqrt_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_ss::<9>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_ss::<10>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_ss::<11>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_sqrt_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_ss::<4>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_ss::<8>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_ss::<9>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_ss::<10>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_ss::<11>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_sqrt_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_sd::<9>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_sd::<10>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_round_sd::<11>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_sqrt_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_sd::<9>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_sd::<10>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_round_sd::<11>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_sqrt_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_sd::<4>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_sd::<8>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_sd::<9>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_sd::<10>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sqrt_round_sd::<11>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_getexp_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_getexp_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_getexp_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_getexp_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_getexp_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_round_ss::<4>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_round_ss::<8>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_getexp_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_getexp_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_getexp_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_getexp_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_getexp_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_round_sd::<4>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_getexp_round_sd::<8>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_scalef_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_ss::<9>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_ss::<10>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_ss::<11>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_scalef_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_ss::<9>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_ss::<10>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_ss::<11>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_scalef_round_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_ss::<4>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_ss::<8>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_ss::<9>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_ss::<10>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_ss::<11>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_scalef_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_sd::<9>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_sd::<10>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_scalef_round_sd::<11>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_scalef_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_sd::<9>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_sd::<10>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_round_sd::<11>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_scalef_round_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_sd::<4>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_sd::<8>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_sd::<9>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_sd::<10>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_scalef_round_sd::<11>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_cvtss_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtss_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_cvtss_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtss_sd(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_cvtsd_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsd_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_cvtsd_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtsd_ss(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundss_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_cvt_roundss_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvt_roundss_sd::<4>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvt_roundss_sd::<8>(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_cvt_roundss_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvt_roundss_sd::<4>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvt_roundss_sd::<8>(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundsd_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_ss::<9>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_ss::<10>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_ss::<11>(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_mask_cvt_roundsd_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvt_roundsd_ss::<4>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvt_roundsd_ss::<8>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvt_roundsd_ss::<9>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvt_roundsd_ss::<10>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvt_roundsd_ss::<11>(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_maskz_cvt_roundsd_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvt_roundsd_ss::<4>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvt_roundsd_ss::<8>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvt_roundsd_ss::<9>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvt_roundsd_ss::<10>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvt_roundsd_ss::<11>(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundss_si32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_si32::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_si32::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_si32::<9>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_si32::<10>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_si32::<11>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundss_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_i32::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_i32::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_i32::<9>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_i32::<10>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_i32::<11>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundss_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u32::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u32::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u32::<9>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u32::<10>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u32::<11>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtss_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtss_i32(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtss_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtss_u32(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundsd_si32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_si32::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_si32::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_si32::<9>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_si32::<10>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_si32::<11>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundsd_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_i32::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_i32::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_i32::<9>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_i32::<10>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_i32::<11>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundsd_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u32::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u32::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u32::<9>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u32::<10>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u32::<11>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtsd_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsd_i32(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtsd_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsd_u32(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundss_si32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_si32::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_si32::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundss_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_i32::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_i32::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundss_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_u32::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_u32::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttss_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttss_i32(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttss_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttss_u32(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundsd_si32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_si32::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_si32::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundsd_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_i32::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_i32::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundsd_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_u32::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_u32::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttsd_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsd_i32(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttsd_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsd_u32(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtss_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtss_u64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtsd_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsd_u64(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttsd_i64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsd_i64(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttsd_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsd_u64(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttss_i64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttss_i64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvttss_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttss_u64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundsd_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u64::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u64::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u64::<9>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u64::<10>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundsd_u64::<11>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvt_roundss_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u64::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u64::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u64::<9>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u64::<10>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvt_roundss_u64::<11>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundsd_si64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_si64::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_si64::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundsd_i64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_i64::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_i64::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundsd_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_u64::<4>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundsd_u64::<8>(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundss_i64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_i64::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_i64::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundss_si64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_si64::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_si64::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512f")]
unsafe fn mm_cvtt_roundss_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_u64::<4>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm_cvtt_roundss_u64::<8>(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_pd::<4>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_pd::<8>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_pd::<9>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_pd::<10>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_pd::<11>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_pd::<9>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_pd::<10>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_pd::<11>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_pd::<4>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_pd::<8>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_pd::<9>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_pd::<10>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_pd::<11>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi64_pd(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi64_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepi64_pd(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepi64_pd(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepi64_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtepi64_pd(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepi64_pd(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtepi64_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtepi64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtepi64_pd(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_ps::<4>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_ps::<8>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_ps::<9>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_ps::<10>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepi64_ps::<11>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_ps::<4>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_ps::<8>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_ps::<9>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_ps::<10>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepi64_ps::<11>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_ps::<4>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_ps::<8>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_ps::<9>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_ps::<10>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepi64_ps::<11>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi64_ps(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi64_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepi64_ps(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepi64_ps(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepi64_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtepi64_ps(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepi64_ps(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtepi64_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtepi64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtepi64_ps(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_pd::<4>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_pd::<8>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_pd::<9>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_pd::<10>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_pd::<11>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_pd::<4>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_pd::<8>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_pd::<9>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_pd::<10>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_pd::<11>(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_pd::<4>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_pd::<8>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_pd::<9>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_pd::<10>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_pd::<11>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepu64_pd(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepu64_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepu64_pd(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepu64_pd(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepu64_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtepu64_pd(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepu64_pd(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtepu64_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtepu64_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtepu64_pd(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_ps::<4>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_ps::<8>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_ps::<9>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_ps::<10>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundepu64_ps::<11>(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_ps::<4>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_ps::<8>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_ps::<9>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_ps::<10>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundepu64_ps::<11>(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_ps::<4>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_ps::<8>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_ps::<9>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_ps::<10>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundepu64_ps::<11>(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepu64_ps(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepu64_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtepu64_ps(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepu64_ps(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepu64_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtepu64_ps(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepu64_ps(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtepu64_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtepu64_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtepu64_ps(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi64::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi64::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi64::<9>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi64::<10>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epi64::<11>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi64::<9>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi64::<10>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epi64::<11>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi64::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi64::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi64::<9>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi64::<10>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epi64::<11>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtpd_epi64(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtpd_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtpd_epi64(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtpd_epi64(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtpd_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtpd_epi64(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtpd_epi64(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpd_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtpd_epi64(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi64::<4>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi64::<8>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi64::<9>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi64::<10>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epi64::<11>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi64::<9>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi64::<10>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epi64::<11>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi64::<4>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi64::<8>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi64::<9>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi64::<10>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epi64::<11>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtps_epi64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtps_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtps_epi64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtps_epi64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtps_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtps_epi64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtps_epi64(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtps_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtps_epi64(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu64::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu64::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu64::<9>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu64::<10>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundpd_epu64::<11>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu64::<9>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu64::<10>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundpd_epu64::<11>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu64::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu64::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu64::<9>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu64::<10>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundpd_epu64::<11>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtpd_epu64(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtpd_epu64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtpd_epu64(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtpd_epu64(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtpd_epu64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtpd_epu64(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtpd_epu64(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpd_epu64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtpd_epu64(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvt_roundps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu64::<4>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu64::<8>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu64::<9>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu64::<10>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvt_roundps_epu64::<11>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvt_roundps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu64::<9>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu64::<10>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvt_roundps_epu64::<11>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvt_roundps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu64::<4>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu64::<8>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu64::<9>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu64::<10>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvt_roundps_epu64::<11>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtps_epu64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtps_epu64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtps_epu64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtps_epu64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtps_epu64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvtps_epu64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtps_epu64(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtps_epu64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtps_epu64(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtt_roundpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epi64::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epi64::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtt_roundpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epi64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epi64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtt_roundpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epi64::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epi64::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttpd_epi64(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttpd_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvttpd_epi64(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvttpd_epi64(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttpd_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvttpd_epi64(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttpd_epi64(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttpd_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvttpd_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttpd_epi64(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtt_roundps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epi64::<4>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epi64::<8>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtt_roundps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epi64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epi64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtt_roundps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epi64::<4>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epi64::<8>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttps_epi64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttps_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvttps_epi64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvttps_epi64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttps_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvttps_epi64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttps_epi64(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttps_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvttps_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttps_epi64(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtt_roundpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epu64::<4>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundpd_epu64::<8>(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtt_roundpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epu64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundpd_epu64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtt_roundpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epu64::<4>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundpd_epu64::<8>(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttpd_epu64(ld::<__m128d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttpd_epu64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvttpd_epu64(m as __mmask8, ld::<__m128d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvttpd_epu64(ld::<__m256d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttpd_epu64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvttpd_epu64(m as __mmask8, ld::<__m256d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttpd_epu64(ld::<__m512d>(&F64[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttpd_epu64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvttpd_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttpd_epu64(m as __mmask8, ld::<__m512d>(&F64[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvtt_roundps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epu64::<4>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    for i in 0..3 {
        let r = unsafe { _mm512_cvtt_roundps_epu64::<8>(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvtt_roundps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epu64::<4>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtt_roundps_epu64::<8>(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvtt_roundps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epu64::<4>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvtt_roundps_epu64::<8>(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttps_epu64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_mask_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttps_epu64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm_maskz_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvttps_epu64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvttps_epu64(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_mask_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttps_epu64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq,avx512vl")]
unsafe fn mm256_maskz_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_cvttps_epu64(m as __mmask8, ld::<__m128>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvttps_epu64(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_mask_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttps_epu64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512dq")]
unsafe fn mm512_maskz_cvttps_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_cvttps_epu64(m as __mmask8, ld::<__m256>(&F32[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_mulhrs_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_mulhrs_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_mask_mulhrs_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_mulhrs_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_maskz_mulhrs_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_mulhrs_epi16(m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_sll_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sll_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_mask_sll_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_sll_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_maskz_sll_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_sll_epi16(m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_srl_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_srl_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_mask_srl_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_srl_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_maskz_srl_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_srl_epi16(m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_sra_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sra_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_mask_sra_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_sra_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_maskz_sra_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_sra_epi16(m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw,avx512vl")]
unsafe fn mm_cvtsepi16_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsepi16_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bw,avx512vl")]
unsafe fn mm_mask_cvtsepi16_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsepi16_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw,avx512vl")]
unsafe fn mm_maskz_cvtsepi16_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtsepi16_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw,avx512vl")]
unsafe fn mm_cvtusepi16_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtusepi16_epi8(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bw,avx512vl")]
unsafe fn mm_mask_cvtusepi16_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtusepi16_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw,avx512vl")]
unsafe fn mm_maskz_cvtusepi16_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_cvtusepi16_epi8(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_mask_cvtsepi16_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtsepi16_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512bw")]
unsafe fn mm512_mask_cvtusepi16_storeu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_cvtusepi16_storeu_epi8(o0.as_mut_ptr().cast(), m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512cd")]
unsafe fn mm512_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_conflict_epi32(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512cd")]
unsafe fn mm512_mask_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_conflict_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd")]
unsafe fn mm512_maskz_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_conflict_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm256_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_conflict_epi32(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm256_mask_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_conflict_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm256_maskz_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_conflict_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_conflict_epi32(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm_mask_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_conflict_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm_maskz_conflict_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_conflict_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd")]
unsafe fn mm512_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_conflict_epi64(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512cd")]
unsafe fn mm512_mask_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_conflict_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd")]
unsafe fn mm512_maskz_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_conflict_epi64(m as __mmask8, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm256_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_conflict_epi64(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm256_mask_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_conflict_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm256_maskz_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_conflict_epi64(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_conflict_epi64(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm_mask_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_conflict_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512cd,avx512vl")]
unsafe fn mm_maskz_conflict_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_conflict_epi64(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi")]
unsafe fn mm512_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_multishift_epi64_epi8(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vbmi")]
unsafe fn mm512_mask_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_multishift_epi64_epi8(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask64, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi")]
unsafe fn mm512_maskz_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_multishift_epi64_epi8(m as __mmask64, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi,avx512vl")]
unsafe fn mm256_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_multishift_epi64_epi8(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vbmi,avx512vl")]
unsafe fn mm256_mask_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_multishift_epi64_epi8(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi,avx512vl")]
unsafe fn mm256_maskz_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_multishift_epi64_epi8(m as __mmask32, ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi,avx512vl")]
unsafe fn mm_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_multishift_epi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vbmi,avx512vl")]
unsafe fn mm_mask_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_multishift_epi64_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi,avx512vl")]
unsafe fn mm_maskz_multishift_epi64_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_multishift_epi64_epi8(m as __mmask16, ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_expandloadu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expandloadu_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_maskz_expandloadu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expandloadu_epi16(m as __mmask32, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_expandloadu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expandloadu_epi16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_maskz_expandloadu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expandloadu_epi16(m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_expandloadu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expandloadu_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_maskz_expandloadu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expandloadu_epi16(m as __mmask8, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_expandloadu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expandloadu_epi8(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask64, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_maskz_expandloadu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expandloadu_epi8(m as __mmask64, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_expandloadu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expandloadu_epi8(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask32, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_maskz_expandloadu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expandloadu_epi8(m as __mmask32, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_expandloadu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expandloadu_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_maskz_expandloadu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expandloadu_epi8(m as __mmask16, SRC.as_ptr().cast()) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_compressstoreu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_compressstoreu_epi16(o0.as_mut_ptr().cast(), m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_compressstoreu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_compressstoreu_epi16(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_compressstoreu_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_compressstoreu_epi16(o0.as_mut_ptr().cast(), m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_compressstoreu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm512_mask_compressstoreu_epi8(o0.as_mut_ptr().cast(), m as __mmask64, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_compressstoreu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm256_mask_compressstoreu_epi8(o0.as_mut_ptr().cast(), m as __mmask32, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_compressstoreu_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let mut o0 = [0u8; 128];
            unsafe { _mm_mask_compressstoreu_epi8(o0.as_mut_ptr().cast(), m as __mmask16, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &o0);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_compress_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_compress_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_maskz_compress_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_compress_epi16(m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_compress_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_compress_epi16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_maskz_compress_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_compress_epi16(m as __mmask16, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_compress_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_compress_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_maskz_compress_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_compress_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_compress_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_compress_epi8(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask64, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_maskz_compress_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_compress_epi8(m as __mmask64, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_compress_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_compress_epi8(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_maskz_compress_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_compress_epi8(m as __mmask32, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_compress_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_compress_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_maskz_compress_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_compress_epi8(m as __mmask16, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_expand_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expand_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_maskz_expand_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expand_epi16(m as __mmask32, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_expand_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expand_epi16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_maskz_expand_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expand_epi16(m as __mmask16, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_expand_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expand_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_maskz_expand_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expand_epi16(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_mask_expand_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_expand_epi8(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask64, ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2")]
unsafe fn mm512_maskz_expand_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_expand_epi8(m as __mmask64, ld::<__m512i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_mask_expand_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_expand_epi8(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm256_maskz_expand_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_expand_epi8(m as __mmask32, ld::<__m256i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_mask_expand_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_expand_epi8(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vbmi2,avx512vl")]
unsafe fn mm_maskz_expand_epi8() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_expand_epi8(m as __mmask16, ld::<__m128i>(&INT[(i + 0) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_dpwssd_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_mask_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_dpwssd_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_maskz_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_dpwssd_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm256_dpwssd_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwssd_avx_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwssd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_mask_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_dpwssd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_maskz_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_dpwssd_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm_dpwssd_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwssd_avx_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwssd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_mask_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_dpwssd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_maskz_dpwssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_dpwssd_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_dpwssds_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_mask_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_dpwssds_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_maskz_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_dpwssds_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm256_dpwssds_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwssds_avx_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwssds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_mask_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_dpwssds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_maskz_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_dpwssds_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm_dpwssds_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwssds_avx_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwssds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_mask_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_dpwssds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_maskz_dpwssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_dpwssds_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_dpbusd_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_mask_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_dpbusd_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_maskz_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_dpbusd_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm256_dpbusd_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbusd_avx_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbusd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_mask_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_dpbusd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_maskz_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_dpbusd_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm_dpbusd_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbusd_avx_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbusd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_mask_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_dpbusd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_maskz_dpbusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_dpbusd_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_dpbusds_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_mask_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_dpbusds_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni")]
unsafe fn mm512_maskz_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_dpbusds_epi32(m as __mmask16, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), ld::<__m512i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm256_dpbusds_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbusds_avx_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbusds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_mask_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_dpbusds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm256_maskz_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_dpbusds_epi32(m as __mmask8, ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnni")]
unsafe fn mm_dpbusds_avx_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbusds_avx_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbusds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_mask_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_dpbusds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vnni,avx512vl")]
unsafe fn mm_maskz_dpbusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_dpbusds_epi32(m as __mmask8, ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm_dpbssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbssd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm256_dpbssd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbssd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm_dpbssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbssds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm256_dpbssds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbssds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm_dpbsud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbsud_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm256_dpbsud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbsud_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm_dpbsuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbsuds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm256_dpbsuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbsuds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm_dpbuud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbuud_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm256_dpbuud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbuud_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm_dpbuuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbuuds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint8")]
unsafe fn mm256_dpbuuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbuuds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm_dpwsud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwsud_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm256_dpwsud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwsud_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm_dpwsuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwsuds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm256_dpwsuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwsuds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm_dpwusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwusd_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm256_dpwusd_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwusd_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm_dpwusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwusds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm256_dpwusds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwusds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm_dpwuud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwuud_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm256_dpwuud_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwuud_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm_dpwuuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpwuuds_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), ld::<__m128i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxvnniint16")]
unsafe fn mm256_dpwuuds_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpwuuds_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), ld::<__m256i>(&INT[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bitalg")]
unsafe fn mm512_bitshuffle_epi64_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_bitshuffle_epi64_mask(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bitalg")]
unsafe fn mm512_mask_bitshuffle_epi64_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_bitshuffle_epi64_mask(m as __mmask64, ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bitalg,avx512vl")]
unsafe fn mm256_bitshuffle_epi64_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_bitshuffle_epi64_mask(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bitalg,avx512vl")]
unsafe fn mm256_mask_bitshuffle_epi64_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_bitshuffle_epi64_mask(m as __mmask32, ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512bitalg,avx512vl")]
unsafe fn mm_bitshuffle_epi64_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_bitshuffle_epi64_mask(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bitalg,avx512vl")]
unsafe fn mm_mask_bitshuffle_epi64_mask() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_bitshuffle_epi64_mask(m as __mmask16, ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512vp2intersect,avx512vl")]
unsafe fn mm_2intersect_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let mut o2: __mmask8 = 0;
        let mut o3: __mmask8 = 0;
        unsafe { _mm_2intersect_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), &mut o2, &mut o3) };
        mix(&mut h, &o2);
        mix(&mut h, &o3);
    }
    h
}
#[target_feature(enable = "avx512vp2intersect,avx512vl")]
unsafe fn mm_2intersect_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let mut o2: __mmask8 = 0;
        let mut o3: __mmask8 = 0;
        unsafe { _mm_2intersect_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), ld::<__m128i>(&INT[(i + 1) % 3]), &mut o2, &mut o3) };
        mix(&mut h, &o2);
        mix(&mut h, &o3);
    }
    h
}
#[target_feature(enable = "avx512vp2intersect,avx512vl")]
unsafe fn mm256_2intersect_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let mut o2: __mmask8 = 0;
        let mut o3: __mmask8 = 0;
        unsafe { _mm256_2intersect_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), &mut o2, &mut o3) };
        mix(&mut h, &o2);
        mix(&mut h, &o3);
    }
    h
}
#[target_feature(enable = "avx512vp2intersect,avx512vl")]
unsafe fn mm256_2intersect_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let mut o2: __mmask8 = 0;
        let mut o3: __mmask8 = 0;
        unsafe { _mm256_2intersect_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), ld::<__m256i>(&INT[(i + 1) % 3]), &mut o2, &mut o3) };
        mix(&mut h, &o2);
        mix(&mut h, &o3);
    }
    h
}
#[target_feature(enable = "avx512vp2intersect,avx512f")]
unsafe fn mm512_2intersect_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let mut o2: __mmask16 = 0;
        let mut o3: __mmask16 = 0;
        unsafe { _mm512_2intersect_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), &mut o2, &mut o3) };
        mix(&mut h, &o2);
        mix(&mut h, &o3);
    }
    h
}
#[target_feature(enable = "avx512vp2intersect,avx512f")]
unsafe fn mm512_2intersect_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let mut o2: __mmask8 = 0;
        let mut o3: __mmask8 = 0;
        unsafe { _mm512_2intersect_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), ld::<__m512i>(&INT[(i + 1) % 3]), &mut o2, &mut o3) };
        mix(&mut h, &o2);
        mix(&mut h, &o3);
    }
    h
}
pub fn run() -> String {
    let mut out = String::new();
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_max_ps {:016x}\n", unsafe { mm512_max_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_max_ps {:016x}\n", unsafe { mm512_mask_max_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_max_ps {:016x}\n", unsafe { mm512_maskz_max_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_max_pd {:016x}\n", unsafe { mm512_max_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_max_pd {:016x}\n", unsafe { mm512_mask_max_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_max_pd {:016x}\n", unsafe { mm512_maskz_max_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_min_ps {:016x}\n", unsafe { mm512_min_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_min_ps {:016x}\n", unsafe { mm512_mask_min_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_min_ps {:016x}\n", unsafe { mm512_maskz_min_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_min_pd {:016x}\n", unsafe { mm512_min_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_min_pd {:016x}\n", unsafe { mm512_mask_min_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_min_pd {:016x}\n", unsafe { mm512_maskz_min_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_rcp14_ps {:016x}\n", unsafe { mm512_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_rcp14_ps {:016x}\n", unsafe { mm512_mask_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_rcp14_ps {:016x}\n", unsafe { mm512_maskz_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_rcp14_ps {:016x}\n", unsafe { mm256_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_rcp14_ps {:016x}\n", unsafe { mm256_mask_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_rcp14_ps {:016x}\n", unsafe { mm256_maskz_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_rcp14_ps {:016x}\n", unsafe { mm_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_rcp14_ps {:016x}\n", unsafe { mm_mask_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_rcp14_ps {:016x}\n", unsafe { mm_maskz_rcp14_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_rcp14_pd {:016x}\n", unsafe { mm512_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_rcp14_pd {:016x}\n", unsafe { mm512_mask_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_rcp14_pd {:016x}\n", unsafe { mm512_maskz_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_rcp14_pd {:016x}\n", unsafe { mm256_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_rcp14_pd {:016x}\n", unsafe { mm256_mask_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_rcp14_pd {:016x}\n", unsafe { mm256_maskz_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_rcp14_pd {:016x}\n", unsafe { mm_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_rcp14_pd {:016x}\n", unsafe { mm_mask_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_rcp14_pd {:016x}\n", unsafe { mm_maskz_rcp14_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_rsqrt14_ps {:016x}\n", unsafe { mm512_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_rsqrt14_ps {:016x}\n", unsafe { mm512_mask_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_rsqrt14_ps {:016x}\n", unsafe { mm512_maskz_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_rsqrt14_ps {:016x}\n", unsafe { mm256_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_rsqrt14_ps {:016x}\n", unsafe { mm256_mask_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_rsqrt14_ps {:016x}\n", unsafe { mm256_maskz_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_rsqrt14_ps {:016x}\n", unsafe { mm_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_rsqrt14_ps {:016x}\n", unsafe { mm_mask_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_rsqrt14_ps {:016x}\n", unsafe { mm_maskz_rsqrt14_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_rsqrt14_pd {:016x}\n", unsafe { mm512_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_rsqrt14_pd {:016x}\n", unsafe { mm512_mask_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_rsqrt14_pd {:016x}\n", unsafe { mm512_maskz_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_rsqrt14_pd {:016x}\n", unsafe { mm256_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_rsqrt14_pd {:016x}\n", unsafe { mm256_mask_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_rsqrt14_pd {:016x}\n", unsafe { mm256_maskz_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_rsqrt14_pd {:016x}\n", unsafe { mm_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_rsqrt14_pd {:016x}\n", unsafe { mm_mask_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_rsqrt14_pd {:016x}\n", unsafe { mm_maskz_rsqrt14_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_getexp_ps {:016x}\n", unsafe { mm512_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_getexp_ps {:016x}\n", unsafe { mm512_mask_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_getexp_ps {:016x}\n", unsafe { mm512_maskz_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_getexp_ps {:016x}\n", unsafe { mm256_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_getexp_ps {:016x}\n", unsafe { mm256_mask_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_getexp_ps {:016x}\n", unsafe { mm256_maskz_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_getexp_ps {:016x}\n", unsafe { mm_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_getexp_ps {:016x}\n", unsafe { mm_mask_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_getexp_ps {:016x}\n", unsafe { mm_maskz_getexp_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_getexp_pd {:016x}\n", unsafe { mm512_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_getexp_pd {:016x}\n", unsafe { mm512_mask_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_getexp_pd {:016x}\n", unsafe { mm512_maskz_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_getexp_pd {:016x}\n", unsafe { mm256_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_getexp_pd {:016x}\n", unsafe { mm256_mask_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_getexp_pd {:016x}\n", unsafe { mm256_maskz_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_getexp_pd {:016x}\n", unsafe { mm_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_getexp_pd {:016x}\n", unsafe { mm_mask_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_getexp_pd {:016x}\n", unsafe { mm_maskz_getexp_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_scalef_ps {:016x}\n", unsafe { mm512_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_scalef_ps {:016x}\n", unsafe { mm512_mask_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_scalef_ps {:016x}\n", unsafe { mm512_maskz_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_scalef_ps {:016x}\n", unsafe { mm256_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_scalef_ps {:016x}\n", unsafe { mm256_mask_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_scalef_ps {:016x}\n", unsafe { mm256_maskz_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_scalef_ps {:016x}\n", unsafe { mm_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_scalef_ps {:016x}\n", unsafe { mm_mask_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_scalef_ps {:016x}\n", unsafe { mm_maskz_scalef_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_scalef_pd {:016x}\n", unsafe { mm512_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_scalef_pd {:016x}\n", unsafe { mm512_mask_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_scalef_pd {:016x}\n", unsafe { mm512_maskz_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_scalef_pd {:016x}\n", unsafe { mm256_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_scalef_pd {:016x}\n", unsafe { mm256_mask_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_scalef_pd {:016x}\n", unsafe { mm256_maskz_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_scalef_pd {:016x}\n", unsafe { mm_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_scalef_pd {:016x}\n", unsafe { mm_mask_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_scalef_pd {:016x}\n", unsafe { mm_maskz_scalef_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_max_round_ps {:016x}\n", unsafe { mm512_max_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_max_round_ps {:016x}\n", unsafe { mm512_mask_max_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_max_round_ps {:016x}\n", unsafe { mm512_maskz_max_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_max_round_pd {:016x}\n", unsafe { mm512_max_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_max_round_pd {:016x}\n", unsafe { mm512_mask_max_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_max_round_pd {:016x}\n", unsafe { mm512_maskz_max_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_min_round_ps {:016x}\n", unsafe { mm512_min_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_min_round_ps {:016x}\n", unsafe { mm512_mask_min_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_min_round_ps {:016x}\n", unsafe { mm512_maskz_min_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_min_round_pd {:016x}\n", unsafe { mm512_min_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_min_round_pd {:016x}\n", unsafe { mm512_mask_min_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_min_round_pd {:016x}\n", unsafe { mm512_maskz_min_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_getexp_round_ps {:016x}\n", unsafe { mm512_getexp_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_getexp_round_ps {:016x}\n", unsafe { mm512_mask_getexp_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_getexp_round_ps {:016x}\n", unsafe { mm512_maskz_getexp_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_getexp_round_pd {:016x}\n", unsafe { mm512_getexp_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_getexp_round_pd {:016x}\n", unsafe { mm512_mask_getexp_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_getexp_round_pd {:016x}\n", unsafe { mm512_maskz_getexp_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_scalef_round_ps {:016x}\n", unsafe { mm512_scalef_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_scalef_round_ps {:016x}\n", unsafe { mm512_mask_scalef_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_scalef_round_ps {:016x}\n", unsafe { mm512_maskz_scalef_round_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_scalef_round_pd {:016x}\n", unsafe { mm512_scalef_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_scalef_round_pd {:016x}\n", unsafe { mm512_mask_scalef_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_scalef_round_pd {:016x}\n", unsafe { mm512_maskz_scalef_round_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtps_epi32 {:016x}\n", unsafe { mm512_cvtps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtps_epi32 {:016x}\n", unsafe { mm512_mask_cvtps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtps_epi32 {:016x}\n", unsafe { mm512_maskz_cvtps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtps_epu32 {:016x}\n", unsafe { mm512_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtps_epu32 {:016x}\n", unsafe { mm512_mask_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtps_epu32 {:016x}\n", unsafe { mm512_maskz_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtps_epu32 {:016x}\n", unsafe { mm256_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtps_epu32 {:016x}\n", unsafe { mm256_mask_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtps_epu32 {:016x}\n", unsafe { mm256_maskz_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtps_epu32 {:016x}\n", unsafe { mm_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtps_epu32 {:016x}\n", unsafe { mm_mask_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtps_epu32 {:016x}\n", unsafe { mm_maskz_cvtps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtps_pd {:016x}\n", unsafe { mm512_cvtps_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtps_pd {:016x}\n", unsafe { mm512_mask_cvtps_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtps_pd {:016x}\n", unsafe { mm512_maskz_cvtps_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtpslo_pd {:016x}\n", unsafe { mm512_cvtpslo_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtpslo_pd {:016x}\n", unsafe { mm512_mask_cvtpslo_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtpd_ps {:016x}\n", unsafe { mm512_cvtpd_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtpd_ps {:016x}\n", unsafe { mm512_mask_cvtpd_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtpd_ps {:016x}\n", unsafe { mm512_maskz_cvtpd_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtpd_ps {:016x}\n", unsafe { mm_mask_cvtpd_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtpd_epi32 {:016x}\n", unsafe { mm512_cvtpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtpd_epi32 {:016x}\n", unsafe { mm512_mask_cvtpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtpd_epi32 {:016x}\n", unsafe { mm512_maskz_cvtpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtpd_epi32 {:016x}\n", unsafe { mm_mask_cvtpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtpd_epu32 {:016x}\n", unsafe { mm512_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtpd_epu32 {:016x}\n", unsafe { mm512_mask_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtpd_epu32 {:016x}\n", unsafe { mm512_maskz_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtpd_epu32 {:016x}\n", unsafe { mm256_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtpd_epu32 {:016x}\n", unsafe { mm256_mask_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtpd_epu32 {:016x}\n", unsafe { mm256_maskz_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtpd_epu32 {:016x}\n", unsafe { mm_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtpd_epu32 {:016x}\n", unsafe { mm_mask_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtpd_epu32 {:016x}\n", unsafe { mm_maskz_cvtpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtpd_pslo {:016x}\n", unsafe { mm512_cvtpd_pslo() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtpd_pslo {:016x}\n", unsafe { mm512_mask_cvtpd_pslo() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi32_epi16 {:016x}\n", unsafe { mm_cvtepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi32_epi16 {:016x}\n", unsafe { mm_mask_cvtepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepi32_epi16 {:016x}\n", unsafe { mm_maskz_cvtepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepi32_epi8 {:016x}\n", unsafe { mm256_cvtepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi32_epi8 {:016x}\n", unsafe { mm256_mask_cvtepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtepi32_epi8 {:016x}\n", unsafe { mm256_maskz_cvtepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi32_epi8 {:016x}\n", unsafe { mm_cvtepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi32_epi8 {:016x}\n", unsafe { mm_mask_cvtepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepi32_epi8 {:016x}\n", unsafe { mm_maskz_cvtepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi64_epi32 {:016x}\n", unsafe { mm_cvtepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_epi32 {:016x}\n", unsafe { mm_mask_cvtepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepi64_epi32 {:016x}\n", unsafe { mm_maskz_cvtepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepi64_epi16 {:016x}\n", unsafe { mm256_cvtepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_epi16 {:016x}\n", unsafe { mm256_mask_cvtepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtepi64_epi16 {:016x}\n", unsafe { mm256_maskz_cvtepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi64_epi16 {:016x}\n", unsafe { mm_cvtepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_epi16 {:016x}\n", unsafe { mm_mask_cvtepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepi64_epi16 {:016x}\n", unsafe { mm_maskz_cvtepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtepi64_epi8 {:016x}\n", unsafe { mm512_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtepi64_epi8 {:016x}\n", unsafe { mm512_mask_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtepi64_epi8 {:016x}\n", unsafe { mm512_maskz_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepi64_epi8 {:016x}\n", unsafe { mm256_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_epi8 {:016x}\n", unsafe { mm256_mask_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtepi64_epi8 {:016x}\n", unsafe { mm256_maskz_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi64_epi8 {:016x}\n", unsafe { mm_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_epi8 {:016x}\n", unsafe { mm_mask_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepi64_epi8 {:016x}\n", unsafe { mm_maskz_cvtepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtsepi32_epi16 {:016x}\n", unsafe { mm512_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi32_epi16 {:016x}\n", unsafe { mm512_mask_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtsepi32_epi16 {:016x}\n", unsafe { mm512_maskz_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtsepi32_epi16 {:016x}\n", unsafe { mm256_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi32_epi16 {:016x}\n", unsafe { mm256_mask_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtsepi32_epi16 {:016x}\n", unsafe { mm256_maskz_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtsepi32_epi16 {:016x}\n", unsafe { mm_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi32_epi16 {:016x}\n", unsafe { mm_mask_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtsepi32_epi16 {:016x}\n", unsafe { mm_maskz_cvtsepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtsepi32_epi8 {:016x}\n", unsafe { mm512_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi32_epi8 {:016x}\n", unsafe { mm512_mask_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtsepi32_epi8 {:016x}\n", unsafe { mm512_maskz_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtsepi32_epi8 {:016x}\n", unsafe { mm256_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi32_epi8 {:016x}\n", unsafe { mm256_mask_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtsepi32_epi8 {:016x}\n", unsafe { mm256_maskz_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtsepi32_epi8 {:016x}\n", unsafe { mm_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi32_epi8 {:016x}\n", unsafe { mm_mask_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtsepi32_epi8 {:016x}\n", unsafe { mm_maskz_cvtsepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtsepi64_epi32 {:016x}\n", unsafe { mm512_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi64_epi32 {:016x}\n", unsafe { mm512_mask_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtsepi64_epi32 {:016x}\n", unsafe { mm512_maskz_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtsepi64_epi32 {:016x}\n", unsafe { mm256_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi64_epi32 {:016x}\n", unsafe { mm256_mask_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtsepi64_epi32 {:016x}\n", unsafe { mm256_maskz_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtsepi64_epi32 {:016x}\n", unsafe { mm_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi64_epi32 {:016x}\n", unsafe { mm_mask_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtsepi64_epi32 {:016x}\n", unsafe { mm_maskz_cvtsepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtsepi64_epi16 {:016x}\n", unsafe { mm512_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi64_epi16 {:016x}\n", unsafe { mm512_mask_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtsepi64_epi16 {:016x}\n", unsafe { mm512_maskz_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtsepi64_epi16 {:016x}\n", unsafe { mm256_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi64_epi16 {:016x}\n", unsafe { mm256_mask_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtsepi64_epi16 {:016x}\n", unsafe { mm256_maskz_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtsepi64_epi16 {:016x}\n", unsafe { mm_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi64_epi16 {:016x}\n", unsafe { mm_mask_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtsepi64_epi16 {:016x}\n", unsafe { mm_maskz_cvtsepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtsepi64_epi8 {:016x}\n", unsafe { mm512_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi64_epi8 {:016x}\n", unsafe { mm512_mask_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtsepi64_epi8 {:016x}\n", unsafe { mm512_maskz_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtsepi64_epi8 {:016x}\n", unsafe { mm256_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi64_epi8 {:016x}\n", unsafe { mm256_mask_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtsepi64_epi8 {:016x}\n", unsafe { mm256_maskz_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtsepi64_epi8 {:016x}\n", unsafe { mm_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi64_epi8 {:016x}\n", unsafe { mm_mask_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtsepi64_epi8 {:016x}\n", unsafe { mm_maskz_cvtsepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtusepi32_epi16 {:016x}\n", unsafe { mm512_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi32_epi16 {:016x}\n", unsafe { mm512_mask_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtusepi32_epi16 {:016x}\n", unsafe { mm512_maskz_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtusepi32_epi16 {:016x}\n", unsafe { mm256_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi32_epi16 {:016x}\n", unsafe { mm256_mask_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtusepi32_epi16 {:016x}\n", unsafe { mm256_maskz_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtusepi32_epi16 {:016x}\n", unsafe { mm_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi32_epi16 {:016x}\n", unsafe { mm_mask_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtusepi32_epi16 {:016x}\n", unsafe { mm_maskz_cvtusepi32_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtusepi32_epi8 {:016x}\n", unsafe { mm512_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi32_epi8 {:016x}\n", unsafe { mm512_mask_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtusepi32_epi8 {:016x}\n", unsafe { mm512_maskz_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtusepi32_epi8 {:016x}\n", unsafe { mm256_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi32_epi8 {:016x}\n", unsafe { mm256_mask_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtusepi32_epi8 {:016x}\n", unsafe { mm256_maskz_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtusepi32_epi8 {:016x}\n", unsafe { mm_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi32_epi8 {:016x}\n", unsafe { mm_mask_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtusepi32_epi8 {:016x}\n", unsafe { mm_maskz_cvtusepi32_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtusepi64_epi32 {:016x}\n", unsafe { mm512_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi64_epi32 {:016x}\n", unsafe { mm512_mask_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtusepi64_epi32 {:016x}\n", unsafe { mm512_maskz_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtusepi64_epi32 {:016x}\n", unsafe { mm256_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi64_epi32 {:016x}\n", unsafe { mm256_mask_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtusepi64_epi32 {:016x}\n", unsafe { mm256_maskz_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtusepi64_epi32 {:016x}\n", unsafe { mm_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi64_epi32 {:016x}\n", unsafe { mm_mask_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtusepi64_epi32 {:016x}\n", unsafe { mm_maskz_cvtusepi64_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtusepi64_epi16 {:016x}\n", unsafe { mm512_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi64_epi16 {:016x}\n", unsafe { mm512_mask_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtusepi64_epi16 {:016x}\n", unsafe { mm512_maskz_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtusepi64_epi16 {:016x}\n", unsafe { mm256_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi64_epi16 {:016x}\n", unsafe { mm256_mask_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtusepi64_epi16 {:016x}\n", unsafe { mm256_maskz_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtusepi64_epi16 {:016x}\n", unsafe { mm_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi64_epi16 {:016x}\n", unsafe { mm_mask_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtusepi64_epi16 {:016x}\n", unsafe { mm_maskz_cvtusepi64_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtusepi64_epi8 {:016x}\n", unsafe { mm512_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi64_epi8 {:016x}\n", unsafe { mm512_mask_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtusepi64_epi8 {:016x}\n", unsafe { mm512_maskz_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtusepi64_epi8 {:016x}\n", unsafe { mm256_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi64_epi8 {:016x}\n", unsafe { mm256_mask_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtusepi64_epi8 {:016x}\n", unsafe { mm256_maskz_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtusepi64_epi8 {:016x}\n", unsafe { mm_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi64_epi8 {:016x}\n", unsafe { mm_mask_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtusepi64_epi8 {:016x}\n", unsafe { mm_maskz_cvtusepi64_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvt_roundps_epi32 {:016x}\n", unsafe { mm512_cvt_roundps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvt_roundps_epi32 {:016x}\n", unsafe { mm512_mask_cvt_roundps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvt_roundps_epi32 {:016x}\n", unsafe { mm512_maskz_cvt_roundps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvt_roundps_epu32 {:016x}\n", unsafe { mm512_cvt_roundps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvt_roundps_epu32 {:016x}\n", unsafe { mm512_mask_cvt_roundps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvt_roundps_epu32 {:016x}\n", unsafe { mm512_maskz_cvt_roundps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvt_roundps_pd {:016x}\n", unsafe { mm512_cvt_roundps_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvt_roundps_pd {:016x}\n", unsafe { mm512_mask_cvt_roundps_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvt_roundps_pd {:016x}\n", unsafe { mm512_maskz_cvt_roundps_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvt_roundpd_epi32 {:016x}\n", unsafe { mm512_cvt_roundpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvt_roundpd_epi32 {:016x}\n", unsafe { mm512_mask_cvt_roundpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvt_roundpd_epi32 {:016x}\n", unsafe { mm512_maskz_cvt_roundpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvt_roundpd_epu32 {:016x}\n", unsafe { mm512_cvt_roundpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvt_roundpd_epu32 {:016x}\n", unsafe { mm512_mask_cvt_roundpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvt_roundpd_epu32 {:016x}\n", unsafe { mm512_maskz_cvt_roundpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvt_roundpd_ps {:016x}\n", unsafe { mm512_cvt_roundpd_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvt_roundpd_ps {:016x}\n", unsafe { mm512_mask_cvt_roundpd_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvt_roundpd_ps {:016x}\n", unsafe { mm512_maskz_cvt_roundpd_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvt_roundph_ps {:016x}\n", unsafe { mm512_cvt_roundph_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvt_roundph_ps {:016x}\n", unsafe { mm512_mask_cvt_roundph_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvt_roundph_ps {:016x}\n", unsafe { mm512_maskz_cvt_roundph_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtph_ps {:016x}\n", unsafe { mm512_cvtph_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtph_ps {:016x}\n", unsafe { mm512_mask_cvtph_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtph_ps {:016x}\n", unsafe { mm512_maskz_cvtph_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtt_roundps_epi32 {:016x}\n", unsafe { mm512_cvtt_roundps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtt_roundps_epi32 {:016x}\n", unsafe { mm512_mask_cvtt_roundps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtt_roundps_epi32 {:016x}\n", unsafe { mm512_maskz_cvtt_roundps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtt_roundps_epu32 {:016x}\n", unsafe { mm512_cvtt_roundps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtt_roundps_epu32 {:016x}\n", unsafe { mm512_mask_cvtt_roundps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtt_roundps_epu32 {:016x}\n", unsafe { mm512_maskz_cvtt_roundps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtt_roundpd_epi32 {:016x}\n", unsafe { mm512_cvtt_roundpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtt_roundpd_epi32 {:016x}\n", unsafe { mm512_mask_cvtt_roundpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtt_roundpd_epi32 {:016x}\n", unsafe { mm512_maskz_cvtt_roundpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtt_roundpd_epu32 {:016x}\n", unsafe { mm512_cvtt_roundpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtt_roundpd_epu32 {:016x}\n", unsafe { mm512_mask_cvtt_roundpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvttps_epi32 {:016x}\n", unsafe { mm512_cvttps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvttps_epi32 {:016x}\n", unsafe { mm512_mask_cvttps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvttps_epi32 {:016x}\n", unsafe { mm512_maskz_cvttps_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvttps_epu32 {:016x}\n", unsafe { mm512_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvttps_epu32 {:016x}\n", unsafe { mm512_mask_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvttps_epu32 {:016x}\n", unsafe { mm512_maskz_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvttps_epu32 {:016x}\n", unsafe { mm256_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttps_epu32 {:016x}\n", unsafe { mm256_mask_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvttps_epu32 {:016x}\n", unsafe { mm256_maskz_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvttps_epu32 {:016x}\n", unsafe { mm_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttps_epu32 {:016x}\n", unsafe { mm_mask_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvttps_epu32 {:016x}\n", unsafe { mm_maskz_cvttps_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvtt_roundpd_epu32 {:016x}\n", unsafe { mm512_maskz_cvtt_roundpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvttpd_epi32 {:016x}\n", unsafe { mm512_cvttpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvttpd_epi32 {:016x}\n", unsafe { mm512_mask_cvttpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvttpd_epi32 {:016x}\n", unsafe { mm512_maskz_cvttpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttpd_epi32 {:016x}\n", unsafe { mm_mask_cvttpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvttpd_epi32 {:016x}\n", unsafe { mm_maskz_cvttpd_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvttpd_epu32 {:016x}\n", unsafe { mm512_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvttpd_epu32 {:016x}\n", unsafe { mm512_mask_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_cvttpd_epu32 {:016x}\n", unsafe { mm512_maskz_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvttpd_epu32 {:016x}\n", unsafe { mm256_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttpd_epu32 {:016x}\n", unsafe { mm256_mask_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvttpd_epu32 {:016x}\n", unsafe { mm256_maskz_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvttpd_epu32 {:016x}\n", unsafe { mm_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttpd_epu32 {:016x}\n", unsafe { mm_mask_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvttpd_epu32 {:016x}\n", unsafe { mm_maskz_cvttpd_epu32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compress_epi32 {:016x}\n", unsafe { mm512_mask_compress_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_compress_epi32 {:016x}\n", unsafe { mm512_maskz_compress_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compress_epi32 {:016x}\n", unsafe { mm256_mask_compress_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_compress_epi32 {:016x}\n", unsafe { mm256_maskz_compress_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compress_epi32 {:016x}\n", unsafe { mm_mask_compress_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_compress_epi32 {:016x}\n", unsafe { mm_maskz_compress_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compress_epi64 {:016x}\n", unsafe { mm512_mask_compress_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_compress_epi64 {:016x}\n", unsafe { mm512_maskz_compress_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compress_epi64 {:016x}\n", unsafe { mm256_mask_compress_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_compress_epi64 {:016x}\n", unsafe { mm256_maskz_compress_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compress_epi64 {:016x}\n", unsafe { mm_mask_compress_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_compress_epi64 {:016x}\n", unsafe { mm_maskz_compress_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compress_ps {:016x}\n", unsafe { mm512_mask_compress_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_compress_ps {:016x}\n", unsafe { mm512_maskz_compress_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compress_ps {:016x}\n", unsafe { mm256_mask_compress_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_compress_ps {:016x}\n", unsafe { mm256_maskz_compress_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compress_ps {:016x}\n", unsafe { mm_mask_compress_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_compress_ps {:016x}\n", unsafe { mm_maskz_compress_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compress_pd {:016x}\n", unsafe { mm512_mask_compress_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_compress_pd {:016x}\n", unsafe { mm512_maskz_compress_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compress_pd {:016x}\n", unsafe { mm256_mask_compress_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_compress_pd {:016x}\n", unsafe { mm256_maskz_compress_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compress_pd {:016x}\n", unsafe { mm_mask_compress_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_compress_pd {:016x}\n", unsafe { mm_maskz_compress_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compressstoreu_epi32 {:016x}\n", unsafe { mm512_mask_compressstoreu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compressstoreu_epi32 {:016x}\n", unsafe { mm256_mask_compressstoreu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compressstoreu_epi32 {:016x}\n", unsafe { mm_mask_compressstoreu_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compressstoreu_epi64 {:016x}\n", unsafe { mm512_mask_compressstoreu_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compressstoreu_epi64 {:016x}\n", unsafe { mm256_mask_compressstoreu_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compressstoreu_epi64 {:016x}\n", unsafe { mm_mask_compressstoreu_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compressstoreu_ps {:016x}\n", unsafe { mm512_mask_compressstoreu_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compressstoreu_ps {:016x}\n", unsafe { mm256_mask_compressstoreu_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compressstoreu_ps {:016x}\n", unsafe { mm_mask_compressstoreu_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_compressstoreu_pd {:016x}\n", unsafe { mm512_mask_compressstoreu_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compressstoreu_pd {:016x}\n", unsafe { mm256_mask_compressstoreu_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compressstoreu_pd {:016x}\n", unsafe { mm_mask_compressstoreu_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expand_epi32 {:016x}\n", unsafe { mm512_mask_expand_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expand_epi32 {:016x}\n", unsafe { mm512_maskz_expand_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expand_epi32 {:016x}\n", unsafe { mm256_mask_expand_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expand_epi32 {:016x}\n", unsafe { mm256_maskz_expand_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expand_epi32 {:016x}\n", unsafe { mm_mask_expand_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expand_epi32 {:016x}\n", unsafe { mm_maskz_expand_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expand_epi64 {:016x}\n", unsafe { mm512_mask_expand_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expand_epi64 {:016x}\n", unsafe { mm512_maskz_expand_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expand_epi64 {:016x}\n", unsafe { mm256_mask_expand_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expand_epi64 {:016x}\n", unsafe { mm256_maskz_expand_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expand_epi64 {:016x}\n", unsafe { mm_mask_expand_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expand_epi64 {:016x}\n", unsafe { mm_maskz_expand_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expand_ps {:016x}\n", unsafe { mm512_mask_expand_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expand_ps {:016x}\n", unsafe { mm512_maskz_expand_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expand_ps {:016x}\n", unsafe { mm256_mask_expand_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expand_ps {:016x}\n", unsafe { mm256_maskz_expand_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expand_ps {:016x}\n", unsafe { mm_mask_expand_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expand_ps {:016x}\n", unsafe { mm_maskz_expand_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expand_pd {:016x}\n", unsafe { mm512_mask_expand_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expand_pd {:016x}\n", unsafe { mm512_maskz_expand_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expand_pd {:016x}\n", unsafe { mm256_mask_expand_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expand_pd {:016x}\n", unsafe { mm256_maskz_expand_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expand_pd {:016x}\n", unsafe { mm_mask_expand_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expand_pd {:016x}\n", unsafe { mm_maskz_expand_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_sll_epi32 {:016x}\n", unsafe { mm512_sll_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_sll_epi32 {:016x}\n", unsafe { mm512_mask_sll_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_sll_epi32 {:016x}\n", unsafe { mm512_maskz_sll_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_srl_epi32 {:016x}\n", unsafe { mm512_srl_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_srl_epi32 {:016x}\n", unsafe { mm512_mask_srl_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_srl_epi32 {:016x}\n", unsafe { mm512_maskz_srl_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_sll_epi64 {:016x}\n", unsafe { mm512_sll_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_sll_epi64 {:016x}\n", unsafe { mm512_mask_sll_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_sll_epi64 {:016x}\n", unsafe { mm512_maskz_sll_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_srl_epi64 {:016x}\n", unsafe { mm512_srl_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_srl_epi64 {:016x}\n", unsafe { mm512_mask_srl_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_srl_epi64 {:016x}\n", unsafe { mm512_maskz_srl_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_sra_epi32 {:016x}\n", unsafe { mm512_sra_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_sra_epi32 {:016x}\n", unsafe { mm512_mask_sra_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_sra_epi32 {:016x}\n", unsafe { mm512_maskz_sra_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_sra_epi64 {:016x}\n", unsafe { mm512_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_sra_epi64 {:016x}\n", unsafe { mm512_mask_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_sra_epi64 {:016x}\n", unsafe { mm512_maskz_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_sra_epi64 {:016x}\n", unsafe { mm256_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_sra_epi64 {:016x}\n", unsafe { mm256_mask_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_sra_epi64 {:016x}\n", unsafe { mm256_maskz_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_sra_epi64 {:016x}\n", unsafe { mm_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_sra_epi64 {:016x}\n", unsafe { mm_mask_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_sra_epi64 {:016x}\n", unsafe { mm_maskz_sra_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_permutevar_ps {:016x}\n", unsafe { mm512_permutevar_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_permutevar_ps {:016x}\n", unsafe { mm512_mask_permutevar_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_permutevar_ps {:016x}\n", unsafe { mm512_maskz_permutevar_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_permutevar_pd {:016x}\n", unsafe { mm512_permutevar_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_permutevar_pd {:016x}\n", unsafe { mm512_mask_permutevar_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_permutevar_pd {:016x}\n", unsafe { mm512_maskz_permutevar_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cmp_ps_mask {:016x}\n", unsafe { mm512_cmp_ps_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cmp_ps_mask {:016x}\n", unsafe { mm512_mask_cmp_ps_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cmp_round_ps_mask {:016x}\n", unsafe { mm512_cmp_round_ps_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cmp_round_ps_mask {:016x}\n", unsafe { mm512_mask_cmp_round_ps_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cmp_pd_mask {:016x}\n", unsafe { mm512_cmp_pd_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cmp_pd_mask {:016x}\n", unsafe { mm512_mask_cmp_pd_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cmp_round_pd_mask {:016x}\n", unsafe { mm512_cmp_round_pd_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cmp_round_pd_mask {:016x}\n", unsafe { mm512_mask_cmp_round_pd_mask() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtepi32_storeu_epi16 {:016x}\n", unsafe { mm512_mask_cvtepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi32_storeu_epi16 {:016x}\n", unsafe { mm256_mask_cvtepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi32_storeu_epi16 {:016x}\n", unsafe { mm_mask_cvtepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi32_storeu_epi16 {:016x}\n", unsafe { mm512_mask_cvtsepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi32_storeu_epi16 {:016x}\n", unsafe { mm256_mask_cvtsepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi32_storeu_epi16 {:016x}\n", unsafe { mm_mask_cvtsepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi32_storeu_epi16 {:016x}\n", unsafe { mm512_mask_cvtusepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi32_storeu_epi16 {:016x}\n", unsafe { mm256_mask_cvtusepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi32_storeu_epi16 {:016x}\n", unsafe { mm_mask_cvtusepi32_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtepi32_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi32_storeu_epi8 {:016x}\n", unsafe { mm256_mask_cvtepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi32_storeu_epi8 {:016x}\n", unsafe { mm_mask_cvtepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi32_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtsepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi32_storeu_epi8 {:016x}\n", unsafe { mm256_mask_cvtsepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi32_storeu_epi8 {:016x}\n", unsafe { mm_mask_cvtsepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi32_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtusepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi32_storeu_epi8 {:016x}\n", unsafe { mm256_mask_cvtusepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi32_storeu_epi8 {:016x}\n", unsafe { mm_mask_cvtusepi32_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtepi64_storeu_epi16 {:016x}\n", unsafe { mm512_mask_cvtepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_storeu_epi16 {:016x}\n", unsafe { mm256_mask_cvtepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_storeu_epi16 {:016x}\n", unsafe { mm_mask_cvtepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi64_storeu_epi16 {:016x}\n", unsafe { mm512_mask_cvtsepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi64_storeu_epi16 {:016x}\n", unsafe { mm256_mask_cvtsepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi64_storeu_epi16 {:016x}\n", unsafe { mm_mask_cvtsepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi64_storeu_epi16 {:016x}\n", unsafe { mm512_mask_cvtusepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi64_storeu_epi16 {:016x}\n", unsafe { mm256_mask_cvtusepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi64_storeu_epi16 {:016x}\n", unsafe { mm_mask_cvtusepi64_storeu_epi16() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtepi64_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_storeu_epi8 {:016x}\n", unsafe { mm256_mask_cvtepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_storeu_epi8 {:016x}\n", unsafe { mm_mask_cvtepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi64_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtsepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi64_storeu_epi8 {:016x}\n", unsafe { mm256_mask_cvtsepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi64_storeu_epi8 {:016x}\n", unsafe { mm_mask_cvtsepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi64_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtusepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi64_storeu_epi8 {:016x}\n", unsafe { mm256_mask_cvtusepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi64_storeu_epi8 {:016x}\n", unsafe { mm_mask_cvtusepi64_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtepi64_storeu_epi32 {:016x}\n", unsafe { mm512_mask_cvtepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_storeu_epi32 {:016x}\n", unsafe { mm256_mask_cvtepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_storeu_epi32 {:016x}\n", unsafe { mm_mask_cvtepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtsepi64_storeu_epi32 {:016x}\n", unsafe { mm512_mask_cvtsepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtsepi64_storeu_epi32 {:016x}\n", unsafe { mm256_mask_cvtsepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi64_storeu_epi32 {:016x}\n", unsafe { mm_mask_cvtsepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_cvtusepi64_storeu_epi32 {:016x}\n", unsafe { mm512_mask_cvtusepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtusepi64_storeu_epi32 {:016x}\n", unsafe { mm256_mask_cvtusepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi64_storeu_epi32 {:016x}\n", unsafe { mm_mask_cvtusepi64_storeu_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expandloadu_epi32 {:016x}\n", unsafe { mm512_mask_expandloadu_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expandloadu_epi32 {:016x}\n", unsafe { mm512_maskz_expandloadu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expandloadu_epi32 {:016x}\n", unsafe { mm256_mask_expandloadu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expandloadu_epi32 {:016x}\n", unsafe { mm256_maskz_expandloadu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expandloadu_epi32 {:016x}\n", unsafe { mm_mask_expandloadu_epi32() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expandloadu_epi32 {:016x}\n", unsafe { mm_maskz_expandloadu_epi32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expandloadu_epi64 {:016x}\n", unsafe { mm512_mask_expandloadu_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expandloadu_epi64 {:016x}\n", unsafe { mm512_maskz_expandloadu_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expandloadu_epi64 {:016x}\n", unsafe { mm256_mask_expandloadu_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expandloadu_epi64 {:016x}\n", unsafe { mm256_maskz_expandloadu_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expandloadu_epi64 {:016x}\n", unsafe { mm_mask_expandloadu_epi64() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expandloadu_epi64 {:016x}\n", unsafe { mm_maskz_expandloadu_epi64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expandloadu_ps {:016x}\n", unsafe { mm512_mask_expandloadu_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expandloadu_ps {:016x}\n", unsafe { mm512_maskz_expandloadu_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expandloadu_ps {:016x}\n", unsafe { mm256_mask_expandloadu_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expandloadu_ps {:016x}\n", unsafe { mm256_maskz_expandloadu_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expandloadu_ps {:016x}\n", unsafe { mm_mask_expandloadu_ps() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expandloadu_ps {:016x}\n", unsafe { mm_maskz_expandloadu_ps() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_mask_expandloadu_pd {:016x}\n", unsafe { mm512_mask_expandloadu_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm512_maskz_expandloadu_pd {:016x}\n", unsafe { mm512_maskz_expandloadu_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expandloadu_pd {:016x}\n", unsafe { mm256_mask_expandloadu_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expandloadu_pd {:016x}\n", unsafe { mm256_maskz_expandloadu_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expandloadu_pd {:016x}\n", unsafe { mm_mask_expandloadu_pd() }); }
    if is_x86_feature_detected!("avx512f") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expandloadu_pd {:016x}\n", unsafe { mm_maskz_expandloadu_pd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_max_ss {:016x}\n", unsafe { mm_mask_max_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_max_ss {:016x}\n", unsafe { mm_maskz_max_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_max_sd {:016x}\n", unsafe { mm_mask_max_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_max_sd {:016x}\n", unsafe { mm_maskz_max_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_min_ss {:016x}\n", unsafe { mm_mask_min_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_min_ss {:016x}\n", unsafe { mm_maskz_min_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_min_sd {:016x}\n", unsafe { mm_mask_min_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_min_sd {:016x}\n", unsafe { mm_maskz_min_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_sqrt_ss {:016x}\n", unsafe { mm_mask_sqrt_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_sqrt_ss {:016x}\n", unsafe { mm_maskz_sqrt_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_sqrt_sd {:016x}\n", unsafe { mm_mask_sqrt_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_sqrt_sd {:016x}\n", unsafe { mm_maskz_sqrt_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_rsqrt14_ss {:016x}\n", unsafe { mm_rsqrt14_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_rsqrt14_ss {:016x}\n", unsafe { mm_mask_rsqrt14_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_rsqrt14_ss {:016x}\n", unsafe { mm_maskz_rsqrt14_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_rsqrt14_sd {:016x}\n", unsafe { mm_rsqrt14_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_rsqrt14_sd {:016x}\n", unsafe { mm_mask_rsqrt14_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_rsqrt14_sd {:016x}\n", unsafe { mm_maskz_rsqrt14_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_rcp14_ss {:016x}\n", unsafe { mm_rcp14_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_rcp14_ss {:016x}\n", unsafe { mm_mask_rcp14_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_rcp14_ss {:016x}\n", unsafe { mm_maskz_rcp14_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_rcp14_sd {:016x}\n", unsafe { mm_rcp14_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_rcp14_sd {:016x}\n", unsafe { mm_mask_rcp14_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_rcp14_sd {:016x}\n", unsafe { mm_maskz_rcp14_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_getexp_ss {:016x}\n", unsafe { mm_getexp_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_getexp_ss {:016x}\n", unsafe { mm_mask_getexp_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_getexp_ss {:016x}\n", unsafe { mm_maskz_getexp_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_getexp_sd {:016x}\n", unsafe { mm_getexp_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_getexp_sd {:016x}\n", unsafe { mm_mask_getexp_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_getexp_sd {:016x}\n", unsafe { mm_maskz_getexp_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_scalef_ss {:016x}\n", unsafe { mm_scalef_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_scalef_ss {:016x}\n", unsafe { mm_mask_scalef_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_scalef_ss {:016x}\n", unsafe { mm_maskz_scalef_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_scalef_sd {:016x}\n", unsafe { mm_scalef_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_scalef_sd {:016x}\n", unsafe { mm_mask_scalef_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_scalef_sd {:016x}\n", unsafe { mm_maskz_scalef_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_max_round_ss {:016x}\n", unsafe { mm_max_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_max_round_ss {:016x}\n", unsafe { mm_mask_max_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_max_round_ss {:016x}\n", unsafe { mm_maskz_max_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_max_round_sd {:016x}\n", unsafe { mm_max_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_max_round_sd {:016x}\n", unsafe { mm_mask_max_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_max_round_sd {:016x}\n", unsafe { mm_maskz_max_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_min_round_ss {:016x}\n", unsafe { mm_min_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_min_round_ss {:016x}\n", unsafe { mm_mask_min_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_min_round_ss {:016x}\n", unsafe { mm_maskz_min_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_min_round_sd {:016x}\n", unsafe { mm_min_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_min_round_sd {:016x}\n", unsafe { mm_mask_min_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_min_round_sd {:016x}\n", unsafe { mm_maskz_min_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_sqrt_round_ss {:016x}\n", unsafe { mm_sqrt_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_sqrt_round_ss {:016x}\n", unsafe { mm_mask_sqrt_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_sqrt_round_ss {:016x}\n", unsafe { mm_maskz_sqrt_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_sqrt_round_sd {:016x}\n", unsafe { mm_sqrt_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_sqrt_round_sd {:016x}\n", unsafe { mm_mask_sqrt_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_sqrt_round_sd {:016x}\n", unsafe { mm_maskz_sqrt_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_getexp_round_ss {:016x}\n", unsafe { mm_getexp_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_getexp_round_ss {:016x}\n", unsafe { mm_mask_getexp_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_getexp_round_ss {:016x}\n", unsafe { mm_maskz_getexp_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_getexp_round_sd {:016x}\n", unsafe { mm_getexp_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_getexp_round_sd {:016x}\n", unsafe { mm_mask_getexp_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_getexp_round_sd {:016x}\n", unsafe { mm_maskz_getexp_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_scalef_round_ss {:016x}\n", unsafe { mm_scalef_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_scalef_round_ss {:016x}\n", unsafe { mm_mask_scalef_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_scalef_round_ss {:016x}\n", unsafe { mm_maskz_scalef_round_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_scalef_round_sd {:016x}\n", unsafe { mm_scalef_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_scalef_round_sd {:016x}\n", unsafe { mm_mask_scalef_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_scalef_round_sd {:016x}\n", unsafe { mm_maskz_scalef_round_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_cvtss_sd {:016x}\n", unsafe { mm_mask_cvtss_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_cvtss_sd {:016x}\n", unsafe { mm_maskz_cvtss_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_cvtsd_ss {:016x}\n", unsafe { mm_mask_cvtsd_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_cvtsd_ss {:016x}\n", unsafe { mm_maskz_cvtsd_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundss_sd {:016x}\n", unsafe { mm_cvt_roundss_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_cvt_roundss_sd {:016x}\n", unsafe { mm_mask_cvt_roundss_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_cvt_roundss_sd {:016x}\n", unsafe { mm_maskz_cvt_roundss_sd() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundsd_ss {:016x}\n", unsafe { mm_cvt_roundsd_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_mask_cvt_roundsd_ss {:016x}\n", unsafe { mm_mask_cvt_roundsd_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_maskz_cvt_roundsd_ss {:016x}\n", unsafe { mm_maskz_cvt_roundsd_ss() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundss_si32 {:016x}\n", unsafe { mm_cvt_roundss_si32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundss_i32 {:016x}\n", unsafe { mm_cvt_roundss_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundss_u32 {:016x}\n", unsafe { mm_cvt_roundss_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtss_i32 {:016x}\n", unsafe { mm_cvtss_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtss_u32 {:016x}\n", unsafe { mm_cvtss_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundsd_si32 {:016x}\n", unsafe { mm_cvt_roundsd_si32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundsd_i32 {:016x}\n", unsafe { mm_cvt_roundsd_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundsd_u32 {:016x}\n", unsafe { mm_cvt_roundsd_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtsd_i32 {:016x}\n", unsafe { mm_cvtsd_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtsd_u32 {:016x}\n", unsafe { mm_cvtsd_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundss_si32 {:016x}\n", unsafe { mm_cvtt_roundss_si32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundss_i32 {:016x}\n", unsafe { mm_cvtt_roundss_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundss_u32 {:016x}\n", unsafe { mm_cvtt_roundss_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttss_i32 {:016x}\n", unsafe { mm_cvttss_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttss_u32 {:016x}\n", unsafe { mm_cvttss_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundsd_si32 {:016x}\n", unsafe { mm_cvtt_roundsd_si32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundsd_i32 {:016x}\n", unsafe { mm_cvtt_roundsd_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundsd_u32 {:016x}\n", unsafe { mm_cvtt_roundsd_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttsd_i32 {:016x}\n", unsafe { mm_cvttsd_i32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttsd_u32 {:016x}\n", unsafe { mm_cvttsd_u32() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtss_u64 {:016x}\n", unsafe { mm_cvtss_u64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtsd_u64 {:016x}\n", unsafe { mm_cvtsd_u64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttsd_i64 {:016x}\n", unsafe { mm_cvttsd_i64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttsd_u64 {:016x}\n", unsafe { mm_cvttsd_u64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttss_i64 {:016x}\n", unsafe { mm_cvttss_i64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvttss_u64 {:016x}\n", unsafe { mm_cvttss_u64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundsd_u64 {:016x}\n", unsafe { mm_cvt_roundsd_u64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvt_roundss_u64 {:016x}\n", unsafe { mm_cvt_roundss_u64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundsd_si64 {:016x}\n", unsafe { mm_cvtt_roundsd_si64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundsd_i64 {:016x}\n", unsafe { mm_cvtt_roundsd_i64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundsd_u64 {:016x}\n", unsafe { mm_cvtt_roundsd_u64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundss_i64 {:016x}\n", unsafe { mm_cvtt_roundss_i64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundss_si64 {:016x}\n", unsafe { mm_cvtt_roundss_si64() }); }
    if is_x86_feature_detected!("avx512f") { out += &format!("_mm_cvtt_roundss_u64 {:016x}\n", unsafe { mm_cvtt_roundss_u64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundepi64_pd {:016x}\n", unsafe { mm512_cvt_roundepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundepi64_pd {:016x}\n", unsafe { mm512_mask_cvt_roundepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundepi64_pd {:016x}\n", unsafe { mm512_maskz_cvt_roundepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi64_pd {:016x}\n", unsafe { mm_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_pd {:016x}\n", unsafe { mm_mask_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepi64_pd {:016x}\n", unsafe { mm_maskz_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepi64_pd {:016x}\n", unsafe { mm256_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_pd {:016x}\n", unsafe { mm256_mask_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtepi64_pd {:016x}\n", unsafe { mm256_maskz_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtepi64_pd {:016x}\n", unsafe { mm512_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtepi64_pd {:016x}\n", unsafe { mm512_mask_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtepi64_pd {:016x}\n", unsafe { mm512_maskz_cvtepi64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundepi64_ps {:016x}\n", unsafe { mm512_cvt_roundepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundepi64_ps {:016x}\n", unsafe { mm512_mask_cvt_roundepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundepi64_ps {:016x}\n", unsafe { mm512_maskz_cvt_roundepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi64_ps {:016x}\n", unsafe { mm_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_ps {:016x}\n", unsafe { mm_mask_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepi64_ps {:016x}\n", unsafe { mm_maskz_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepi64_ps {:016x}\n", unsafe { mm256_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_ps {:016x}\n", unsafe { mm256_mask_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtepi64_ps {:016x}\n", unsafe { mm256_maskz_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtepi64_ps {:016x}\n", unsafe { mm512_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtepi64_ps {:016x}\n", unsafe { mm512_mask_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtepi64_ps {:016x}\n", unsafe { mm512_maskz_cvtepi64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundepu64_pd {:016x}\n", unsafe { mm512_cvt_roundepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundepu64_pd {:016x}\n", unsafe { mm512_mask_cvt_roundepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundepu64_pd {:016x}\n", unsafe { mm512_maskz_cvt_roundepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepu64_pd {:016x}\n", unsafe { mm_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepu64_pd {:016x}\n", unsafe { mm_mask_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepu64_pd {:016x}\n", unsafe { mm_maskz_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepu64_pd {:016x}\n", unsafe { mm256_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepu64_pd {:016x}\n", unsafe { mm256_mask_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtepu64_pd {:016x}\n", unsafe { mm256_maskz_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtepu64_pd {:016x}\n", unsafe { mm512_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtepu64_pd {:016x}\n", unsafe { mm512_mask_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtepu64_pd {:016x}\n", unsafe { mm512_maskz_cvtepu64_pd() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundepu64_ps {:016x}\n", unsafe { mm512_cvt_roundepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundepu64_ps {:016x}\n", unsafe { mm512_mask_cvt_roundepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundepu64_ps {:016x}\n", unsafe { mm512_maskz_cvt_roundepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepu64_ps {:016x}\n", unsafe { mm_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepu64_ps {:016x}\n", unsafe { mm_mask_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtepu64_ps {:016x}\n", unsafe { mm_maskz_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepu64_ps {:016x}\n", unsafe { mm256_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepu64_ps {:016x}\n", unsafe { mm256_mask_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtepu64_ps {:016x}\n", unsafe { mm256_maskz_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtepu64_ps {:016x}\n", unsafe { mm512_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtepu64_ps {:016x}\n", unsafe { mm512_mask_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtepu64_ps {:016x}\n", unsafe { mm512_maskz_cvtepu64_ps() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundpd_epi64 {:016x}\n", unsafe { mm512_cvt_roundpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundpd_epi64 {:016x}\n", unsafe { mm512_mask_cvt_roundpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundpd_epi64 {:016x}\n", unsafe { mm512_maskz_cvt_roundpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtpd_epi64 {:016x}\n", unsafe { mm_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtpd_epi64 {:016x}\n", unsafe { mm_mask_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtpd_epi64 {:016x}\n", unsafe { mm_maskz_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtpd_epi64 {:016x}\n", unsafe { mm256_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtpd_epi64 {:016x}\n", unsafe { mm256_mask_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtpd_epi64 {:016x}\n", unsafe { mm256_maskz_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtpd_epi64 {:016x}\n", unsafe { mm512_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtpd_epi64 {:016x}\n", unsafe { mm512_mask_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtpd_epi64 {:016x}\n", unsafe { mm512_maskz_cvtpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundps_epi64 {:016x}\n", unsafe { mm512_cvt_roundps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundps_epi64 {:016x}\n", unsafe { mm512_mask_cvt_roundps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundps_epi64 {:016x}\n", unsafe { mm512_maskz_cvt_roundps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtps_epi64 {:016x}\n", unsafe { mm_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtps_epi64 {:016x}\n", unsafe { mm_mask_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtps_epi64 {:016x}\n", unsafe { mm_maskz_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtps_epi64 {:016x}\n", unsafe { mm256_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtps_epi64 {:016x}\n", unsafe { mm256_mask_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtps_epi64 {:016x}\n", unsafe { mm256_maskz_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtps_epi64 {:016x}\n", unsafe { mm512_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtps_epi64 {:016x}\n", unsafe { mm512_mask_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtps_epi64 {:016x}\n", unsafe { mm512_maskz_cvtps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundpd_epu64 {:016x}\n", unsafe { mm512_cvt_roundpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundpd_epu64 {:016x}\n", unsafe { mm512_mask_cvt_roundpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundpd_epu64 {:016x}\n", unsafe { mm512_maskz_cvt_roundpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtpd_epu64 {:016x}\n", unsafe { mm_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtpd_epu64 {:016x}\n", unsafe { mm_mask_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtpd_epu64 {:016x}\n", unsafe { mm_maskz_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtpd_epu64 {:016x}\n", unsafe { mm256_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtpd_epu64 {:016x}\n", unsafe { mm256_mask_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtpd_epu64 {:016x}\n", unsafe { mm256_maskz_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtpd_epu64 {:016x}\n", unsafe { mm512_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtpd_epu64 {:016x}\n", unsafe { mm512_mask_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtpd_epu64 {:016x}\n", unsafe { mm512_maskz_cvtpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvt_roundps_epu64 {:016x}\n", unsafe { mm512_cvt_roundps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvt_roundps_epu64 {:016x}\n", unsafe { mm512_mask_cvt_roundps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvt_roundps_epu64 {:016x}\n", unsafe { mm512_maskz_cvt_roundps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtps_epu64 {:016x}\n", unsafe { mm_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtps_epu64 {:016x}\n", unsafe { mm_mask_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtps_epu64 {:016x}\n", unsafe { mm_maskz_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtps_epu64 {:016x}\n", unsafe { mm256_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtps_epu64 {:016x}\n", unsafe { mm256_mask_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvtps_epu64 {:016x}\n", unsafe { mm256_maskz_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtps_epu64 {:016x}\n", unsafe { mm512_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtps_epu64 {:016x}\n", unsafe { mm512_mask_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtps_epu64 {:016x}\n", unsafe { mm512_maskz_cvtps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtt_roundpd_epi64 {:016x}\n", unsafe { mm512_cvtt_roundpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtt_roundpd_epi64 {:016x}\n", unsafe { mm512_mask_cvtt_roundpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtt_roundpd_epi64 {:016x}\n", unsafe { mm512_maskz_cvtt_roundpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvttpd_epi64 {:016x}\n", unsafe { mm_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttpd_epi64 {:016x}\n", unsafe { mm_mask_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvttpd_epi64 {:016x}\n", unsafe { mm_maskz_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvttpd_epi64 {:016x}\n", unsafe { mm256_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttpd_epi64 {:016x}\n", unsafe { mm256_mask_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvttpd_epi64 {:016x}\n", unsafe { mm256_maskz_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvttpd_epi64 {:016x}\n", unsafe { mm512_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvttpd_epi64 {:016x}\n", unsafe { mm512_mask_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvttpd_epi64 {:016x}\n", unsafe { mm512_maskz_cvttpd_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtt_roundps_epi64 {:016x}\n", unsafe { mm512_cvtt_roundps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtt_roundps_epi64 {:016x}\n", unsafe { mm512_mask_cvtt_roundps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtt_roundps_epi64 {:016x}\n", unsafe { mm512_maskz_cvtt_roundps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvttps_epi64 {:016x}\n", unsafe { mm_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttps_epi64 {:016x}\n", unsafe { mm_mask_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvttps_epi64 {:016x}\n", unsafe { mm_maskz_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvttps_epi64 {:016x}\n", unsafe { mm256_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttps_epi64 {:016x}\n", unsafe { mm256_mask_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvttps_epi64 {:016x}\n", unsafe { mm256_maskz_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvttps_epi64 {:016x}\n", unsafe { mm512_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvttps_epi64 {:016x}\n", unsafe { mm512_mask_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvttps_epi64 {:016x}\n", unsafe { mm512_maskz_cvttps_epi64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtt_roundpd_epu64 {:016x}\n", unsafe { mm512_cvtt_roundpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtt_roundpd_epu64 {:016x}\n", unsafe { mm512_mask_cvtt_roundpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtt_roundpd_epu64 {:016x}\n", unsafe { mm512_maskz_cvtt_roundpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvttpd_epu64 {:016x}\n", unsafe { mm_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttpd_epu64 {:016x}\n", unsafe { mm_mask_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvttpd_epu64 {:016x}\n", unsafe { mm_maskz_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvttpd_epu64 {:016x}\n", unsafe { mm256_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttpd_epu64 {:016x}\n", unsafe { mm256_mask_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvttpd_epu64 {:016x}\n", unsafe { mm256_maskz_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvttpd_epu64 {:016x}\n", unsafe { mm512_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvttpd_epu64 {:016x}\n", unsafe { mm512_mask_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvttpd_epu64 {:016x}\n", unsafe { mm512_maskz_cvttpd_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvtt_roundps_epu64 {:016x}\n", unsafe { mm512_cvtt_roundps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvtt_roundps_epu64 {:016x}\n", unsafe { mm512_mask_cvtt_roundps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvtt_roundps_epu64 {:016x}\n", unsafe { mm512_maskz_cvtt_roundps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvttps_epu64 {:016x}\n", unsafe { mm_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttps_epu64 {:016x}\n", unsafe { mm_mask_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvttps_epu64 {:016x}\n", unsafe { mm_maskz_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvttps_epu64 {:016x}\n", unsafe { mm256_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttps_epu64 {:016x}\n", unsafe { mm256_mask_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_cvttps_epu64 {:016x}\n", unsafe { mm256_maskz_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_cvttps_epu64 {:016x}\n", unsafe { mm512_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_mask_cvttps_epu64 {:016x}\n", unsafe { mm512_mask_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512dq") { out += &format!("_mm512_maskz_cvttps_epu64 {:016x}\n", unsafe { mm512_maskz_cvttps_epu64() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_mulhrs_epi16 {:016x}\n", unsafe { mm512_mulhrs_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_mask_mulhrs_epi16 {:016x}\n", unsafe { mm512_mask_mulhrs_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_maskz_mulhrs_epi16 {:016x}\n", unsafe { mm512_maskz_mulhrs_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_sll_epi16 {:016x}\n", unsafe { mm512_sll_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_mask_sll_epi16 {:016x}\n", unsafe { mm512_mask_sll_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_maskz_sll_epi16 {:016x}\n", unsafe { mm512_maskz_sll_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_srl_epi16 {:016x}\n", unsafe { mm512_srl_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_mask_srl_epi16 {:016x}\n", unsafe { mm512_mask_srl_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_maskz_srl_epi16 {:016x}\n", unsafe { mm512_maskz_srl_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_sra_epi16 {:016x}\n", unsafe { mm512_sra_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_mask_sra_epi16 {:016x}\n", unsafe { mm512_mask_sra_epi16() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_maskz_sra_epi16 {:016x}\n", unsafe { mm512_maskz_sra_epi16() }); }
    if is_x86_feature_detected!("avx512bw") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtsepi16_epi8 {:016x}\n", unsafe { mm_cvtsepi16_epi8() }); }
    if is_x86_feature_detected!("avx512bw") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtsepi16_epi8 {:016x}\n", unsafe { mm_mask_cvtsepi16_epi8() }); }
    if is_x86_feature_detected!("avx512bw") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtsepi16_epi8 {:016x}\n", unsafe { mm_maskz_cvtsepi16_epi8() }); }
    if is_x86_feature_detected!("avx512bw") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtusepi16_epi8 {:016x}\n", unsafe { mm_cvtusepi16_epi8() }); }
    if is_x86_feature_detected!("avx512bw") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtusepi16_epi8 {:016x}\n", unsafe { mm_mask_cvtusepi16_epi8() }); }
    if is_x86_feature_detected!("avx512bw") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_cvtusepi16_epi8 {:016x}\n", unsafe { mm_maskz_cvtusepi16_epi8() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_mask_cvtsepi16_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtsepi16_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512bw") { out += &format!("_mm512_mask_cvtusepi16_storeu_epi8 {:016x}\n", unsafe { mm512_mask_cvtusepi16_storeu_epi8() }); }
    if is_x86_feature_detected!("avx512cd") { out += &format!("_mm512_conflict_epi32 {:016x}\n", unsafe { mm512_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") { out += &format!("_mm512_mask_conflict_epi32 {:016x}\n", unsafe { mm512_mask_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") { out += &format!("_mm512_maskz_conflict_epi32 {:016x}\n", unsafe { mm512_maskz_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_conflict_epi32 {:016x}\n", unsafe { mm256_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_conflict_epi32 {:016x}\n", unsafe { mm256_mask_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_conflict_epi32 {:016x}\n", unsafe { mm256_maskz_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_conflict_epi32 {:016x}\n", unsafe { mm_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_conflict_epi32 {:016x}\n", unsafe { mm_mask_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_conflict_epi32 {:016x}\n", unsafe { mm_maskz_conflict_epi32() }); }
    if is_x86_feature_detected!("avx512cd") { out += &format!("_mm512_conflict_epi64 {:016x}\n", unsafe { mm512_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") { out += &format!("_mm512_mask_conflict_epi64 {:016x}\n", unsafe { mm512_mask_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") { out += &format!("_mm512_maskz_conflict_epi64 {:016x}\n", unsafe { mm512_maskz_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_conflict_epi64 {:016x}\n", unsafe { mm256_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_conflict_epi64 {:016x}\n", unsafe { mm256_mask_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_conflict_epi64 {:016x}\n", unsafe { mm256_maskz_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_conflict_epi64 {:016x}\n", unsafe { mm_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_conflict_epi64 {:016x}\n", unsafe { mm_mask_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512cd") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_conflict_epi64 {:016x}\n", unsafe { mm_maskz_conflict_epi64() }); }
    if is_x86_feature_detected!("avx512vbmi") { out += &format!("_mm512_multishift_epi64_epi8 {:016x}\n", unsafe { mm512_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") { out += &format!("_mm512_mask_multishift_epi64_epi8 {:016x}\n", unsafe { mm512_mask_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") { out += &format!("_mm512_maskz_multishift_epi64_epi8 {:016x}\n", unsafe { mm512_maskz_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_multishift_epi64_epi8 {:016x}\n", unsafe { mm256_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_multishift_epi64_epi8 {:016x}\n", unsafe { mm256_mask_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_multishift_epi64_epi8 {:016x}\n", unsafe { mm256_maskz_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_multishift_epi64_epi8 {:016x}\n", unsafe { mm_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_multishift_epi64_epi8 {:016x}\n", unsafe { mm_mask_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_multishift_epi64_epi8 {:016x}\n", unsafe { mm_maskz_multishift_epi64_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_expandloadu_epi16 {:016x}\n", unsafe { mm512_mask_expandloadu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_maskz_expandloadu_epi16 {:016x}\n", unsafe { mm512_maskz_expandloadu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expandloadu_epi16 {:016x}\n", unsafe { mm256_mask_expandloadu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expandloadu_epi16 {:016x}\n", unsafe { mm256_maskz_expandloadu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expandloadu_epi16 {:016x}\n", unsafe { mm_mask_expandloadu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expandloadu_epi16 {:016x}\n", unsafe { mm_maskz_expandloadu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_expandloadu_epi8 {:016x}\n", unsafe { mm512_mask_expandloadu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_maskz_expandloadu_epi8 {:016x}\n", unsafe { mm512_maskz_expandloadu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expandloadu_epi8 {:016x}\n", unsafe { mm256_mask_expandloadu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expandloadu_epi8 {:016x}\n", unsafe { mm256_maskz_expandloadu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expandloadu_epi8 {:016x}\n", unsafe { mm_mask_expandloadu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expandloadu_epi8 {:016x}\n", unsafe { mm_maskz_expandloadu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_compressstoreu_epi16 {:016x}\n", unsafe { mm512_mask_compressstoreu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compressstoreu_epi16 {:016x}\n", unsafe { mm256_mask_compressstoreu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compressstoreu_epi16 {:016x}\n", unsafe { mm_mask_compressstoreu_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_compressstoreu_epi8 {:016x}\n", unsafe { mm512_mask_compressstoreu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compressstoreu_epi8 {:016x}\n", unsafe { mm256_mask_compressstoreu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compressstoreu_epi8 {:016x}\n", unsafe { mm_mask_compressstoreu_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_compress_epi16 {:016x}\n", unsafe { mm512_mask_compress_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_maskz_compress_epi16 {:016x}\n", unsafe { mm512_maskz_compress_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compress_epi16 {:016x}\n", unsafe { mm256_mask_compress_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_compress_epi16 {:016x}\n", unsafe { mm256_maskz_compress_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compress_epi16 {:016x}\n", unsafe { mm_mask_compress_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_compress_epi16 {:016x}\n", unsafe { mm_maskz_compress_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_compress_epi8 {:016x}\n", unsafe { mm512_mask_compress_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_maskz_compress_epi8 {:016x}\n", unsafe { mm512_maskz_compress_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_compress_epi8 {:016x}\n", unsafe { mm256_mask_compress_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_compress_epi8 {:016x}\n", unsafe { mm256_maskz_compress_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_compress_epi8 {:016x}\n", unsafe { mm_mask_compress_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_compress_epi8 {:016x}\n", unsafe { mm_maskz_compress_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_expand_epi16 {:016x}\n", unsafe { mm512_mask_expand_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_maskz_expand_epi16 {:016x}\n", unsafe { mm512_maskz_expand_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expand_epi16 {:016x}\n", unsafe { mm256_mask_expand_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expand_epi16 {:016x}\n", unsafe { mm256_maskz_expand_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expand_epi16 {:016x}\n", unsafe { mm_mask_expand_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expand_epi16 {:016x}\n", unsafe { mm_maskz_expand_epi16() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_mask_expand_epi8 {:016x}\n", unsafe { mm512_mask_expand_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") { out += &format!("_mm512_maskz_expand_epi8 {:016x}\n", unsafe { mm512_maskz_expand_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_expand_epi8 {:016x}\n", unsafe { mm256_mask_expand_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_expand_epi8 {:016x}\n", unsafe { mm256_maskz_expand_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_expand_epi8 {:016x}\n", unsafe { mm_mask_expand_epi8() }); }
    if is_x86_feature_detected!("avx512vbmi2") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_expand_epi8 {:016x}\n", unsafe { mm_maskz_expand_epi8() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_dpwssd_epi32 {:016x}\n", unsafe { mm512_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_mask_dpwssd_epi32 {:016x}\n", unsafe { mm512_mask_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_maskz_dpwssd_epi32 {:016x}\n", unsafe { mm512_maskz_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm256_dpwssd_avx_epi32 {:016x}\n", unsafe { mm256_dpwssd_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_dpwssd_epi32 {:016x}\n", unsafe { mm256_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_dpwssd_epi32 {:016x}\n", unsafe { mm256_mask_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_dpwssd_epi32 {:016x}\n", unsafe { mm256_maskz_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm_dpwssd_avx_epi32 {:016x}\n", unsafe { mm_dpwssd_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_dpwssd_epi32 {:016x}\n", unsafe { mm_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_dpwssd_epi32 {:016x}\n", unsafe { mm_mask_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_dpwssd_epi32 {:016x}\n", unsafe { mm_maskz_dpwssd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_dpwssds_epi32 {:016x}\n", unsafe { mm512_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_mask_dpwssds_epi32 {:016x}\n", unsafe { mm512_mask_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_maskz_dpwssds_epi32 {:016x}\n", unsafe { mm512_maskz_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm256_dpwssds_avx_epi32 {:016x}\n", unsafe { mm256_dpwssds_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_dpwssds_epi32 {:016x}\n", unsafe { mm256_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_dpwssds_epi32 {:016x}\n", unsafe { mm256_mask_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_dpwssds_epi32 {:016x}\n", unsafe { mm256_maskz_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm_dpwssds_avx_epi32 {:016x}\n", unsafe { mm_dpwssds_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_dpwssds_epi32 {:016x}\n", unsafe { mm_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_dpwssds_epi32 {:016x}\n", unsafe { mm_mask_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_dpwssds_epi32 {:016x}\n", unsafe { mm_maskz_dpwssds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_dpbusd_epi32 {:016x}\n", unsafe { mm512_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_mask_dpbusd_epi32 {:016x}\n", unsafe { mm512_mask_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_maskz_dpbusd_epi32 {:016x}\n", unsafe { mm512_maskz_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm256_dpbusd_avx_epi32 {:016x}\n", unsafe { mm256_dpbusd_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_dpbusd_epi32 {:016x}\n", unsafe { mm256_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_dpbusd_epi32 {:016x}\n", unsafe { mm256_mask_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_dpbusd_epi32 {:016x}\n", unsafe { mm256_maskz_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm_dpbusd_avx_epi32 {:016x}\n", unsafe { mm_dpbusd_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_dpbusd_epi32 {:016x}\n", unsafe { mm_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_dpbusd_epi32 {:016x}\n", unsafe { mm_mask_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_dpbusd_epi32 {:016x}\n", unsafe { mm_maskz_dpbusd_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_dpbusds_epi32 {:016x}\n", unsafe { mm512_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_mask_dpbusds_epi32 {:016x}\n", unsafe { mm512_mask_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") { out += &format!("_mm512_maskz_dpbusds_epi32 {:016x}\n", unsafe { mm512_maskz_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm256_dpbusds_avx_epi32 {:016x}\n", unsafe { mm256_dpbusds_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_dpbusds_epi32 {:016x}\n", unsafe { mm256_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_dpbusds_epi32 {:016x}\n", unsafe { mm256_mask_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_dpbusds_epi32 {:016x}\n", unsafe { mm256_maskz_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avxvnni") { out += &format!("_mm_dpbusds_avx_epi32 {:016x}\n", unsafe { mm_dpbusds_avx_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_dpbusds_epi32 {:016x}\n", unsafe { mm_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_dpbusds_epi32 {:016x}\n", unsafe { mm_mask_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avx512vnni") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_dpbusds_epi32 {:016x}\n", unsafe { mm_maskz_dpbusds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm_dpbssd_epi32 {:016x}\n", unsafe { mm_dpbssd_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm256_dpbssd_epi32 {:016x}\n", unsafe { mm256_dpbssd_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm_dpbssds_epi32 {:016x}\n", unsafe { mm_dpbssds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm256_dpbssds_epi32 {:016x}\n", unsafe { mm256_dpbssds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm_dpbsud_epi32 {:016x}\n", unsafe { mm_dpbsud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm256_dpbsud_epi32 {:016x}\n", unsafe { mm256_dpbsud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm_dpbsuds_epi32 {:016x}\n", unsafe { mm_dpbsuds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm256_dpbsuds_epi32 {:016x}\n", unsafe { mm256_dpbsuds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm_dpbuud_epi32 {:016x}\n", unsafe { mm_dpbuud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm256_dpbuud_epi32 {:016x}\n", unsafe { mm256_dpbuud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm_dpbuuds_epi32 {:016x}\n", unsafe { mm_dpbuuds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint8") { out += &format!("_mm256_dpbuuds_epi32 {:016x}\n", unsafe { mm256_dpbuuds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm_dpwsud_epi32 {:016x}\n", unsafe { mm_dpwsud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm256_dpwsud_epi32 {:016x}\n", unsafe { mm256_dpwsud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm_dpwsuds_epi32 {:016x}\n", unsafe { mm_dpwsuds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm256_dpwsuds_epi32 {:016x}\n", unsafe { mm256_dpwsuds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm_dpwusd_epi32 {:016x}\n", unsafe { mm_dpwusd_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm256_dpwusd_epi32 {:016x}\n", unsafe { mm256_dpwusd_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm_dpwusds_epi32 {:016x}\n", unsafe { mm_dpwusds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm256_dpwusds_epi32 {:016x}\n", unsafe { mm256_dpwusds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm_dpwuud_epi32 {:016x}\n", unsafe { mm_dpwuud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm256_dpwuud_epi32 {:016x}\n", unsafe { mm256_dpwuud_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm_dpwuuds_epi32 {:016x}\n", unsafe { mm_dpwuuds_epi32() }); }
    if is_x86_feature_detected!("avxvnniint16") { out += &format!("_mm256_dpwuuds_epi32 {:016x}\n", unsafe { mm256_dpwuuds_epi32() }); }
    if is_x86_feature_detected!("avx512bitalg") { out += &format!("_mm512_bitshuffle_epi64_mask {:016x}\n", unsafe { mm512_bitshuffle_epi64_mask() }); }
    if is_x86_feature_detected!("avx512bitalg") { out += &format!("_mm512_mask_bitshuffle_epi64_mask {:016x}\n", unsafe { mm512_mask_bitshuffle_epi64_mask() }); }
    if is_x86_feature_detected!("avx512bitalg") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_bitshuffle_epi64_mask {:016x}\n", unsafe { mm256_bitshuffle_epi64_mask() }); }
    if is_x86_feature_detected!("avx512bitalg") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_bitshuffle_epi64_mask {:016x}\n", unsafe { mm256_mask_bitshuffle_epi64_mask() }); }
    if is_x86_feature_detected!("avx512bitalg") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_bitshuffle_epi64_mask {:016x}\n", unsafe { mm_bitshuffle_epi64_mask() }); }
    if is_x86_feature_detected!("avx512bitalg") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_bitshuffle_epi64_mask {:016x}\n", unsafe { mm_mask_bitshuffle_epi64_mask() }); }
    if is_x86_feature_detected!("avx512vp2intersect") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_2intersect_epi32 {:016x}\n", unsafe { mm_2intersect_epi32() }); }
    if is_x86_feature_detected!("avx512vp2intersect") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_2intersect_epi64 {:016x}\n", unsafe { mm_2intersect_epi64() }); }
    if is_x86_feature_detected!("avx512vp2intersect") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_2intersect_epi32 {:016x}\n", unsafe { mm256_2intersect_epi32() }); }
    if is_x86_feature_detected!("avx512vp2intersect") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_2intersect_epi64 {:016x}\n", unsafe { mm256_2intersect_epi64() }); }
    if is_x86_feature_detected!("avx512vp2intersect") && is_x86_feature_detected!("avx512f") { out += &format!("_mm512_2intersect_epi32 {:016x}\n", unsafe { mm512_2intersect_epi32() }); }
    if is_x86_feature_detected!("avx512vp2intersect") && is_x86_feature_detected!("avx512f") { out += &format!("_mm512_2intersect_epi64 {:016x}\n", unsafe { mm512_2intersect_epi64() }); }
    out
}
