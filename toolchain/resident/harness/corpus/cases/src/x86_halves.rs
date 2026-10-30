//! The half-precision and bfloat16 functions of `core::arch` on x86-64 whose instructions are
//! LLVM intrinsics or AVX512-FP16's arithmetic: F16C's conversions, AVX512-FP16's arithmetic,
//! conversions in every width and to and from integers, comparisons, complex products and
//! estimates, AVX512-BF16's conversions and dot products, and AVX-NE-CONVERT's loads. Each is
//! called on halves, bfloat16s, floats and integers at the edges (NaNs with payloads, quiet and
//! signalling, infinities, signed zeros, subnormals, ties, values past the integers' range)
//! under masks of none, all, alternate and single lanes; what each returns is hashed. Each runs
//! only where the CPU has its features. Generated from `core::arch`'s sources.
#![allow(clippy::all)]
use core::arch::x86_64::*;
use std::hint::black_box;

const H: [[u8; 64]; 3] = [
    halves([0x7e01, 0x7c01, 0xfe00, 0x8000, 0x0000, 0x7c00, 0xfc00, 0x0001, 0x83ff, 0x0400, 0x3c00, 0xbe00,
            0x4100, 0xc100, 0x7bff, 0xfbff, 0x3555, 0x7d00, 0x6800, 0x6801, 0x5bfe, 0x1400, 0x3800, 0xb800,
            0x4248, 0x7a00, 0xd3ff, 0x2e66, 0x77ff, 0x0200, 0xfd01, 0x3c01]),
    halves([0x3800, 0xb800, 0x3e00, 0x7bff, 0xfbff, 0x0003, 0x4700, 0xc800, 0x7d55, 0xfe34, 0x7800, 0xbc00,
            0x2e66, 0x5640, 0x4c00, 0x8000, 0x3c00, 0x7c00, 0x0001, 0xfc00, 0x6bff, 0x4000, 0x3c01, 0xc000,
            0x7e00, 0x0000, 0x4600, 0x8400, 0x3400, 0xb400, 0x5800, 0xffff]),
    halves([0x4280, 0xc7c0, 0x70e2, 0x8a3d, 0x7bff, 0x7bfe, 0x0010, 0x4000, 0x3a00, 0xc300, 0x6400, 0x6c00,
            0x0400, 0xc8c0, 0x5bfc, 0x7e00, 0x3c00, 0xbc00, 0x7c01, 0x0000, 0x3800, 0x4500, 0x2400, 0xa400,
            0x5000, 0xd000, 0x03ff, 0x8001, 0x7a12, 0x3bff, 0x4001, 0xfe01]),
];
const B: [[u8; 64]; 3] = [
    halves([0x7fc1, 0x7f81, 0xffc0, 0x8000, 0x0000, 0x7f80, 0xff80, 0x0001, 0x807f, 0x0080, 0x3f80, 0xbfc0,
            0x4020, 0xc020, 0x7f7f, 0xff7f, 0x3eab, 0x7fa0, 0x4f00, 0x4f01, 0x477f, 0x3a80, 0x3f00, 0xbf00,
            0x4148, 0x7e00, 0xc77f, 0x3dcd, 0x5eff, 0x0040, 0xffa1, 0x3f81]),
    halves([0x3f00, 0xbf00, 0x3fc0, 0x7f7f, 0xff7f, 0x0003, 0x40e0, 0xc100, 0x7fd5, 0xffb4, 0x7f00, 0xbf80,
            0x3dcd, 0x42c8, 0x4180, 0x8000, 0x3f80, 0x7f80, 0x0001, 0xff80, 0x4b7f, 0x4000, 0x3f81, 0xc000,
            0x7fc0, 0x0000, 0x40c0, 0x8080, 0x3e80, 0xbe80, 0x4300, 0xffff]),
    halves([0x4050, 0xc0f8, 0x5e1c, 0x8147, 0x7f7f, 0x7f7e, 0x0010, 0x4000, 0x3f40, 0xc060, 0x4c80, 0x4d80,
            0x0080, 0xc118, 0x477f, 0x7fc0, 0x3f80, 0xbf80, 0x7f81, 0x0000, 0x3f00, 0x40a0, 0x3c80, 0xbc80,
            0x4a00, 0xca00, 0x007f, 0x8001, 0x5f42, 0x3f7f, 0x4001, 0xffc1]),
];
const F32: [[u8; 64]; 3] = [
    floats([f32::NAN, -0.0, 0.0, f32::INFINITY, f32::NEG_INFINITY, 1.0e-45, -1.1754942e-38, 65504.0,
            1.0, -1.5, 2.5, -2.5, 65520.0, 65519.0, 6.0e-8, 2.9802322e-8]),
    floats([0.5, -0.5, 1.5, 3.0e38, -3.0e38, 1.0e-40, 7.0, -8.0, f32::from_bits(0x7fa0_0001),
            f32::from_bits(0xffc0_1234), 1.0009766, -1.0, 0.1, 100.0, 1.0004883, -0.0]),
    floats([3.25, -7.75, 1.0e10, -1.0e-10, 65504.0, 65536.0, 6.1035156e-5, 2.0, f32::from_bits(0x3f80_8000),
            f32::from_bits(0x3f81_8000), 8388609.0, 33554436.0, 1.0e-38, -9.5, 255.5, f32::NAN]),
];
const F64: [[u8; 64]; 3] = [
    doubles([f64::NAN, -0.0, f64::INFINITY, f64::NEG_INFINITY, 5.0e-324, 65504.0, 2.5, 65520.0]),
    doubles([0.5, -1.5, 65519.99, -6.0e-8, 1.00048828125, 1.000488281250001, f64::from_bits(0x7ff4_0000_0000_0001),
             1.0e300]),
    doubles([3.0e38, 2.9802322387695312e-8, 1.0e-39, -0.75, 6.103515625e-5, -3.25, 1.0e-320, f64::NAN]),
];
const INT: [[u8; 64]; 3] = [
    ints([1 << 63, i64::MAX as u64, u64::MAX, 0, 0x7fff_ffff_8000_0000, 0x0000_7fff_ffff_8000, 0x80ff_7f00_0180_ff7f, 0x1234_5678_9abc_def0]),
    ints([0x00ff_0100_ff7f_0080, -0x8000_0001i64 as u64, 0x0000_ffe0_0000_fff0, 0x7f80_807f_fe02_00ff, 1, -2i64 as u64, 0x0000_ffff_0001_0000, 65520]),
    ints([0x0801_0800_0fff_1001, 0x0003_0002_0001_0000, 0x0807_0605_0403_0201, -0x1234_5678i64 as u64, 0x8000_0000_0000_0001, 65504, 65535, 0x7fff_7fff_7fff_7fff]),
];
const MASKS: [u64; 7] = [0, u64::MAX, 0x5555_5555_5555_5555, 0xaaaa_aaaa_aaaa_aaaa, 1, 1 << 63 | 1 << 31 | 1 << 15 | 1 << 7, 0x0f0f_00ff_f0f0_ff00];
const I32: [i32; 6] = [0, -1, i32::MIN, i32::MAX, 65520, 2049];
const U32: [u32; 5] = [0, 1, u32::MAX, 65504, 4097];
const I64: [i64; 6] = [0, -65519, i64::MIN, i64::MAX, (1 << 40) + 1, 2051];
const U64: [u64; 5] = [0, 1, u64::MAX, 65536, (1 << 53) + 3];

const fn halves(a: [u16; 32]) -> [u8; 64] { unsafe { core::mem::transmute(a) } }
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

#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvti64_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for s in I64 {
            let r = unsafe { _mm_cvti64_sh(ld::<__m128h>(&H[(i + 0) % 3]), s) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvtu64_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for s in U64 {
            let r = unsafe { _mm_cvtu64_sh(ld::<__m128h>(&H[(i + 0) % 3]), s) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvtsh_i64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsh_i64(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvtsh_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsh_u64(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvttsh_i64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsh_i64(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvttsh_u64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsh_u64(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_comieq_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_comieq_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_comige_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_comige_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_comigt_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_comigt_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_comile_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_comile_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_comilt_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_comilt_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_comineq_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_comineq_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_ucomieq_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_ucomieq_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_ucomige_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_ucomige_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_ucomigt_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_ucomigt_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_ucomile_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_ucomile_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_ucomilt_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_ucomilt_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_ucomineq_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_ucomineq_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_add_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_add_ph(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_add_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_add_ph(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_add_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_add_ph(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_add_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_add_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_add_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_add_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_maskz_add_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_add_sh(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_sub_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_sub_ph(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_sub_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_sub_ph(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_sub_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sub_ph(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_sub_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_sub_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_sub_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sub_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_maskz_sub_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_sub_sh(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mul_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_mul_ph(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mul_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_mul_ph(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mul_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_mul_ph(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mul_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_mul_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_mul_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_mul_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_maskz_mul_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_mul_sh(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_div_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_div_ph(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_div_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_div_ph(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_div_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_div_ph(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_div_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_div_sh(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_div_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_div_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_maskz_div_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_div_sh(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_mul_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_mul_pch(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_mul_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_mul_pch(ld::<__m256h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m256h>(&H[(i + 1) % 3]), ld::<__m256h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_mul_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_mul_pch(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_mul_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_mul_sch(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cmul_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cmul_pch(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cmul_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cmul_pch(ld::<__m256h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m256h>(&H[(i + 1) % 3]), ld::<__m256h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cmul_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cmul_pch(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_cmul_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cmul_sch(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask3_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask3_fmadd_pch(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3]), m as __mmask8) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_maskz_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_fmadd_pch(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask3_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask3_fmadd_pch(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3]), ld::<__m256h>(&H[(i + 2) % 3]), m as __mmask8) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_maskz_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_fmadd_pch(m as __mmask8, ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3]), ld::<__m256h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_fmadd_pch(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_fmadd_pch(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask3_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask3_fmadd_pch(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3]), m as __mmask16) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_maskz_fmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_fmadd_pch(m as __mmask16, ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_fmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_fmadd_sch(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_fmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_fmadd_sch(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask3_fmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask3_fmadd_sch(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3]), m as __mmask8) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_maskz_fmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_fmadd_sch(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask3_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask3_fcmadd_pch(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3]), m as __mmask8) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_maskz_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_fcmadd_pch(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask3_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask3_fcmadd_pch(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3]), ld::<__m256h>(&H[(i + 2) % 3]), m as __mmask8) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_maskz_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_maskz_fcmadd_pch(m as __mmask8, ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3]), ld::<__m256h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_fcmadd_pch(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_fcmadd_pch(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask3_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask3_fcmadd_pch(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3]), m as __mmask16) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_maskz_fcmadd_pch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_maskz_fcmadd_pch(m as __mmask16, ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_fcmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_fcmadd_sch(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_fcmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_fcmadd_sch(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask3_fcmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask3_fcmadd_sch(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3]), m as __mmask8) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_maskz_fcmadd_sch() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_maskz_fcmadd_sch(m as __mmask8, ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_rcp_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rcp_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_rcp_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_rcp_ph(ld::<__m256h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_rcp_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_rcp_ph(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_rcp_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rcp_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_rsqrt_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rsqrt_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_rsqrt_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_rsqrt_ph(ld::<__m256h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_rsqrt_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_rsqrt_ph(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_rsqrt_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_rsqrt_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_sqrt_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_sqrt_ph(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_sqrt_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_sqrt_ph(ld::<__m256h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_sqrt_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_sqrt_ph(ld::<__m512h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_sqrt_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_sqrt_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_max_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_max_ph(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_max_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_max_ph(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_max_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_max_ph(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_max_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_max_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_min_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_min_ph(ld::<__m128h>(&H[(i + 0) % 3]), ld::<__m128h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_min_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_min_ph(ld::<__m256h>(&H[(i + 0) % 3]), ld::<__m256h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_min_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_min_ph(ld::<__m512h>(&H[(i + 0) % 3]), ld::<__m512h>(&H[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_min_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_min_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_getexp_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_getexp_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_getexp_ph(ld::<__m256h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_getexp_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_getexp_ph(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_getexp_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_getexp_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_scalef_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_scalef_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_scalef_ph(ld::<__m256h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3]), ld::<__m256h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_scalef_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_scalef_ph(ld::<__m512h>(&H[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3]), ld::<__m512h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_scalef_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_scalef_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_reduce_add_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_reduce_add_ph(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r.to_bits());
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_reduce_mul_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_reduce_mul_ph(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r.to_bits());
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_cvtepi16_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepi16_ph(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_cvtepi16_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepi16_ph(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_cvtepi16_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepi16_ph(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_cvtepu16_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtepu16_ph(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_cvtepu16_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepu16_ph(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_cvtepu16_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepu16_ph(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtepi32_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi32_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_cvtepi32_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepi32_ph(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_cvtepi32_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepi32_ph(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvti32_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for s in I32 {
            let r = unsafe { _mm_cvti32_sh(ld::<__m128h>(&H[(i + 0) % 3]), s) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtepu32_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepu32_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_cvtepu32_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtepu32_ph(ld::<__m256i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_cvtepu32_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepu32_ph(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvtu32_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for s in U32 {
            let r = unsafe { _mm_cvtu32_sh(ld::<__m128h>(&H[(i + 0) % 3]), s) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtepi64_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepi64_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtepi64_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepi64_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_cvtepi64_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepi64_ph(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtepu64_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtepu64_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtepu64_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtepu64_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m256i>(&INT[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_cvtepu64_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtepu64_ph(ld::<__m512i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtxps_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtxps_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtxps_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtxps_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m256>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtxps_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtxps_ph(ld::<__m256h>(&H[(i + 0) % 3]), m as __mmask16, ld::<__m512>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_cvtss_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtss_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128>(&F32[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtpd_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtpd_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtpd_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtpd_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m256d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtpd_ph() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtpd_ph(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m512d>(&F64[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_cvtsd_sh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsd_sh(ld::<__m128h>(&H[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3]), ld::<__m128d>(&F64[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtph_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtph_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtph_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtph_epi16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtph_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtph_epu16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtph_epu16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtph_epu16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtph_epu16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtph_epu16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_epu16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvttph_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttph_epi16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvttph_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttph_epi16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvttph_epi16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttph_epi16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvttph_epu16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttph_epu16(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvttph_epu16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttph_epu16(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvttph_epu16() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttph_epu16(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask32, ld::<__m512h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtph_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtph_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtph_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtph_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtph_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvtsh_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsh_i32(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtph_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtph_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtph_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtph_epu32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtph_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_epu32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvtsh_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtsh_u32(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvttph_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttph_epi32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvttph_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttph_epi32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvttph_epi32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttph_epi32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvttsh_i32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsh_i32(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvttph_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttph_epu32(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvttph_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttph_epu32(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvttph_epu32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttph_epu32(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_cvttsh_u32() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvttsh_u32(ld::<__m128h>(&H[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtph_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtph_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtph_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtph_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtph_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtph_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtph_epu64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtph_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtph_epu64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtph_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_epu64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvttph_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttph_epi64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvttph_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttph_epi64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvttph_epi64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttph_epi64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvttph_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvttph_epu64(ld::<__m128i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvttph_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvttph_epu64(ld::<__m256i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvttph_epu64() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvttph_epu64(ld::<__m512i>(&INT[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtxph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtxph_ps(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtxph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtxph_ps(ld::<__m256>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtxph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtxph_ps(ld::<__m512>(&F32[(i + 0) % 3]), m as __mmask16, ld::<__m256h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_cvtsh_ss() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsh_ss(ld::<__m128>(&F32[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm_mask_cvtph_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtph_pd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16,avx512vl")]
unsafe fn mm256_mask_cvtph_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm256_mask_cvtph_pd(ld::<__m256d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm512_mask_cvtph_pd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm512_mask_cvtph_pd(ld::<__m512d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128h>(&H[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avx512fp16")]
unsafe fn mm_mask_cvtsh_sd() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtsh_sd(ld::<__m128d>(&F64[(i + 0) % 3]), m as __mmask8, ld::<__m128d>(&F64[(i + 1) % 3]), ld::<__m128h>(&H[(i + 2) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm_bcstnebf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_bcstnebf16_ps(black_box(&B[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm256_bcstnebf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_bcstnebf16_ps(black_box(&B[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm_bcstnesh_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_bcstnesh_ps(black_box(&H[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm256_bcstnesh_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_bcstnesh_ps(black_box(&H[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm_cvtneebf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtneebf16_ps(black_box(&B[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm256_cvtneebf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtneebf16_ps(black_box(&B[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm_cvtneeph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtneeph_ps(black_box(&H[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm256_cvtneeph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtneeph_ps(black_box(&H[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm_cvtneobf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtneobf16_ps(black_box(&B[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm256_cvtneobf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtneobf16_ps(black_box(&B[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm_cvtneoph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtneoph_ps(black_box(&H[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm256_cvtneoph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtneoph_ps(black_box(&H[(i + 0) % 3]).as_ptr().cast()) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm_cvtneps_avx_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtneps_avx_pbh(ld::<__m128>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avxneconvert")]
unsafe fn mm256_cvtneps_avx_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtneps_avx_pbh(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512vl")]
unsafe fn mm_cvtne2ps_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtne2ps_pbh(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512vl")]
unsafe fn mm256_cvtne2ps_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtne2ps_pbh(ld::<__m256>(&F32[(i + 0) % 3]), ld::<__m256>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512f")]
unsafe fn mm512_cvtne2ps_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtne2ps_pbh(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512>(&F32[(i + 1) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512vl")]
unsafe fn mm256_cvtneps_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtneps_pbh(ld::<__m256>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512f")]
unsafe fn mm512_cvtneps_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_cvtneps_pbh(ld::<__m512>(&F32[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512vl")]
unsafe fn mm_dpbf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_dpbf16_ps(ld::<__m128>(&F32[(i + 0) % 3]), ld::<__m128bh>(&B[(i + 1) % 3]), ld::<__m128bh>(&B[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512vl")]
unsafe fn mm256_dpbf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_dpbf16_ps(ld::<__m256>(&F32[(i + 0) % 3]), ld::<__m256bh>(&B[(i + 1) % 3]), ld::<__m256bh>(&B[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512f")]
unsafe fn mm512_dpbf16_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm512_dpbf16_ps(ld::<__m512>(&F32[(i + 0) % 3]), ld::<__m512bh>(&B[(i + 1) % 3]), ld::<__m512bh>(&B[(i + 2) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "avx512bf16,avx512vl")]
unsafe fn mm_mask_cvtneps_pbh() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        for m in MASKS {
            let r = unsafe { _mm_mask_cvtneps_pbh(ld::<__m128bh>(&B[(i + 0) % 3]), m as __mmask8, ld::<__m128>(&F32[(i + 1) % 3])) };
            mix(&mut h, &r);
        }
    }
    h
}
#[target_feature(enable = "f16c")]
unsafe fn mm_cvtph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm_cvtph_ps(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}
#[target_feature(enable = "f16c")]
unsafe fn mm256_cvtph_ps() -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325;
    for i in 0..3 {
        let r = unsafe { _mm256_cvtph_ps(ld::<__m128i>(&INT[(i + 0) % 3])) };
        mix(&mut h, &r);
    }
    h
}

pub fn run() -> String {
    let mut out = String::new();
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvti64_sh {:016x}\n", unsafe { mm_cvti64_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvtu64_sh {:016x}\n", unsafe { mm_cvtu64_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvtsh_i64 {:016x}\n", unsafe { mm_cvtsh_i64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvtsh_u64 {:016x}\n", unsafe { mm_cvtsh_u64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvttsh_i64 {:016x}\n", unsafe { mm_cvttsh_i64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvttsh_u64 {:016x}\n", unsafe { mm_cvttsh_u64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_comieq_sh {:016x}\n", unsafe { mm_comieq_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_comige_sh {:016x}\n", unsafe { mm_comige_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_comigt_sh {:016x}\n", unsafe { mm_comigt_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_comile_sh {:016x}\n", unsafe { mm_comile_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_comilt_sh {:016x}\n", unsafe { mm_comilt_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_comineq_sh {:016x}\n", unsafe { mm_comineq_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_ucomieq_sh {:016x}\n", unsafe { mm_ucomieq_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_ucomige_sh {:016x}\n", unsafe { mm_ucomige_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_ucomigt_sh {:016x}\n", unsafe { mm_ucomigt_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_ucomile_sh {:016x}\n", unsafe { mm_ucomile_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_ucomilt_sh {:016x}\n", unsafe { mm_ucomilt_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_ucomineq_sh {:016x}\n", unsafe { mm_ucomineq_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_add_ph {:016x}\n", unsafe { mm_add_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_add_ph {:016x}\n", unsafe { mm256_add_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_add_ph {:016x}\n", unsafe { mm512_add_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_add_sh {:016x}\n", unsafe { mm_add_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_add_sh {:016x}\n", unsafe { mm_mask_add_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_maskz_add_sh {:016x}\n", unsafe { mm_maskz_add_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_sub_ph {:016x}\n", unsafe { mm_sub_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_sub_ph {:016x}\n", unsafe { mm256_sub_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_sub_ph {:016x}\n", unsafe { mm512_sub_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_sub_sh {:016x}\n", unsafe { mm_sub_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_sub_sh {:016x}\n", unsafe { mm_mask_sub_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_maskz_sub_sh {:016x}\n", unsafe { mm_maskz_sub_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mul_ph {:016x}\n", unsafe { mm_mul_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mul_ph {:016x}\n", unsafe { mm256_mul_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mul_ph {:016x}\n", unsafe { mm512_mul_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mul_sh {:016x}\n", unsafe { mm_mul_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_mul_sh {:016x}\n", unsafe { mm_mask_mul_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_maskz_mul_sh {:016x}\n", unsafe { mm_maskz_mul_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_div_ph {:016x}\n", unsafe { mm_div_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_div_ph {:016x}\n", unsafe { mm256_div_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_div_ph {:016x}\n", unsafe { mm512_div_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_div_sh {:016x}\n", unsafe { mm_div_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_div_sh {:016x}\n", unsafe { mm_mask_div_sh() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_maskz_div_sh {:016x}\n", unsafe { mm_maskz_div_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_mul_pch {:016x}\n", unsafe { mm_mask_mul_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_mul_pch {:016x}\n", unsafe { mm256_mask_mul_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_mul_pch {:016x}\n", unsafe { mm512_mask_mul_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_mul_sch {:016x}\n", unsafe { mm_mask_mul_sch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cmul_pch {:016x}\n", unsafe { mm_mask_cmul_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cmul_pch {:016x}\n", unsafe { mm256_mask_cmul_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cmul_pch {:016x}\n", unsafe { mm512_mask_cmul_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_cmul_sch {:016x}\n", unsafe { mm_mask_cmul_sch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask3_fmadd_pch {:016x}\n", unsafe { mm_mask3_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_fmadd_pch {:016x}\n", unsafe { mm_maskz_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask3_fmadd_pch {:016x}\n", unsafe { mm256_mask3_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_fmadd_pch {:016x}\n", unsafe { mm256_maskz_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_fmadd_pch {:016x}\n", unsafe { mm512_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_fmadd_pch {:016x}\n", unsafe { mm512_mask_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask3_fmadd_pch {:016x}\n", unsafe { mm512_mask3_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_maskz_fmadd_pch {:016x}\n", unsafe { mm512_maskz_fmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_fmadd_sch {:016x}\n", unsafe { mm_fmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_fmadd_sch {:016x}\n", unsafe { mm_mask_fmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask3_fmadd_sch {:016x}\n", unsafe { mm_mask3_fmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_maskz_fmadd_sch {:016x}\n", unsafe { mm_maskz_fmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask3_fcmadd_pch {:016x}\n", unsafe { mm_mask3_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_maskz_fcmadd_pch {:016x}\n", unsafe { mm_maskz_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask3_fcmadd_pch {:016x}\n", unsafe { mm256_mask3_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_maskz_fcmadd_pch {:016x}\n", unsafe { mm256_maskz_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_fcmadd_pch {:016x}\n", unsafe { mm512_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_fcmadd_pch {:016x}\n", unsafe { mm512_mask_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask3_fcmadd_pch {:016x}\n", unsafe { mm512_mask3_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_maskz_fcmadd_pch {:016x}\n", unsafe { mm512_maskz_fcmadd_pch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_fcmadd_sch {:016x}\n", unsafe { mm_fcmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_fcmadd_sch {:016x}\n", unsafe { mm_mask_fcmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask3_fcmadd_sch {:016x}\n", unsafe { mm_mask3_fcmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_maskz_fcmadd_sch {:016x}\n", unsafe { mm_maskz_fcmadd_sch() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_rcp_ph {:016x}\n", unsafe { mm_mask_rcp_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_rcp_ph {:016x}\n", unsafe { mm256_mask_rcp_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_rcp_ph {:016x}\n", unsafe { mm512_mask_rcp_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_rcp_sh {:016x}\n", unsafe { mm_mask_rcp_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_rsqrt_ph {:016x}\n", unsafe { mm_mask_rsqrt_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_rsqrt_ph {:016x}\n", unsafe { mm256_mask_rsqrt_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_rsqrt_ph {:016x}\n", unsafe { mm512_mask_rsqrt_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_rsqrt_sh {:016x}\n", unsafe { mm_mask_rsqrt_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_sqrt_ph {:016x}\n", unsafe { mm_sqrt_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_sqrt_ph {:016x}\n", unsafe { mm256_sqrt_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_sqrt_ph {:016x}\n", unsafe { mm512_sqrt_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_sqrt_sh {:016x}\n", unsafe { mm_mask_sqrt_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_max_ph {:016x}\n", unsafe { mm_max_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_max_ph {:016x}\n", unsafe { mm256_max_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_max_ph {:016x}\n", unsafe { mm512_max_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_max_sh {:016x}\n", unsafe { mm_mask_max_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_min_ph {:016x}\n", unsafe { mm_min_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_min_ph {:016x}\n", unsafe { mm256_min_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_min_ph {:016x}\n", unsafe { mm512_min_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_min_sh {:016x}\n", unsafe { mm_mask_min_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_getexp_ph {:016x}\n", unsafe { mm_mask_getexp_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_getexp_ph {:016x}\n", unsafe { mm256_mask_getexp_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_getexp_ph {:016x}\n", unsafe { mm512_mask_getexp_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_getexp_sh {:016x}\n", unsafe { mm_mask_getexp_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_scalef_ph {:016x}\n", unsafe { mm_mask_scalef_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_scalef_ph {:016x}\n", unsafe { mm256_mask_scalef_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_scalef_ph {:016x}\n", unsafe { mm512_mask_scalef_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_scalef_sh {:016x}\n", unsafe { mm_mask_scalef_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_reduce_add_ph {:016x}\n", unsafe { mm_reduce_add_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_reduce_mul_ph {:016x}\n", unsafe { mm_reduce_mul_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepi16_ph {:016x}\n", unsafe { mm_cvtepi16_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepi16_ph {:016x}\n", unsafe { mm256_cvtepi16_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_cvtepi16_ph {:016x}\n", unsafe { mm512_cvtepi16_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtepu16_ph {:016x}\n", unsafe { mm_cvtepu16_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepu16_ph {:016x}\n", unsafe { mm256_cvtepu16_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_cvtepu16_ph {:016x}\n", unsafe { mm512_cvtepu16_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi32_ph {:016x}\n", unsafe { mm_mask_cvtepi32_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepi32_ph {:016x}\n", unsafe { mm256_cvtepi32_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_cvtepi32_ph {:016x}\n", unsafe { mm512_cvtepi32_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvti32_sh {:016x}\n", unsafe { mm_cvti32_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepu32_ph {:016x}\n", unsafe { mm_mask_cvtepu32_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtepu32_ph {:016x}\n", unsafe { mm256_cvtepu32_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_cvtepu32_ph {:016x}\n", unsafe { mm512_cvtepu32_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvtu32_sh {:016x}\n", unsafe { mm_cvtu32_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepi64_ph {:016x}\n", unsafe { mm_mask_cvtepi64_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepi64_ph {:016x}\n", unsafe { mm256_mask_cvtepi64_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_cvtepi64_ph {:016x}\n", unsafe { mm512_cvtepi64_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtepu64_ph {:016x}\n", unsafe { mm_mask_cvtepu64_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtepu64_ph {:016x}\n", unsafe { mm256_mask_cvtepu64_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_cvtepu64_ph {:016x}\n", unsafe { mm512_cvtepu64_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtxps_ph {:016x}\n", unsafe { mm_mask_cvtxps_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtxps_ph {:016x}\n", unsafe { mm256_mask_cvtxps_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtxps_ph {:016x}\n", unsafe { mm512_mask_cvtxps_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_cvtss_sh {:016x}\n", unsafe { mm_mask_cvtss_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtpd_ph {:016x}\n", unsafe { mm_mask_cvtpd_ph() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtpd_ph {:016x}\n", unsafe { mm256_mask_cvtpd_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtpd_ph {:016x}\n", unsafe { mm512_mask_cvtpd_ph() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_cvtsd_sh {:016x}\n", unsafe { mm_mask_cvtsd_sh() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtph_epi16 {:016x}\n", unsafe { mm_mask_cvtph_epi16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtph_epi16 {:016x}\n", unsafe { mm256_mask_cvtph_epi16() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtph_epi16 {:016x}\n", unsafe { mm512_mask_cvtph_epi16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtph_epu16 {:016x}\n", unsafe { mm_mask_cvtph_epu16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtph_epu16 {:016x}\n", unsafe { mm256_mask_cvtph_epu16() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtph_epu16 {:016x}\n", unsafe { mm512_mask_cvtph_epu16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttph_epi16 {:016x}\n", unsafe { mm_mask_cvttph_epi16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttph_epi16 {:016x}\n", unsafe { mm256_mask_cvttph_epi16() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvttph_epi16 {:016x}\n", unsafe { mm512_mask_cvttph_epi16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttph_epu16 {:016x}\n", unsafe { mm_mask_cvttph_epu16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttph_epu16 {:016x}\n", unsafe { mm256_mask_cvttph_epu16() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvttph_epu16 {:016x}\n", unsafe { mm512_mask_cvttph_epu16() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtph_epi32 {:016x}\n", unsafe { mm_mask_cvtph_epi32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtph_epi32 {:016x}\n", unsafe { mm256_mask_cvtph_epi32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtph_epi32 {:016x}\n", unsafe { mm512_mask_cvtph_epi32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvtsh_i32 {:016x}\n", unsafe { mm_cvtsh_i32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtph_epu32 {:016x}\n", unsafe { mm_mask_cvtph_epu32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtph_epu32 {:016x}\n", unsafe { mm256_mask_cvtph_epu32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtph_epu32 {:016x}\n", unsafe { mm512_mask_cvtph_epu32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvtsh_u32 {:016x}\n", unsafe { mm_cvtsh_u32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttph_epi32 {:016x}\n", unsafe { mm_mask_cvttph_epi32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttph_epi32 {:016x}\n", unsafe { mm256_mask_cvttph_epi32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvttph_epi32 {:016x}\n", unsafe { mm512_mask_cvttph_epi32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvttsh_i32 {:016x}\n", unsafe { mm_cvttsh_i32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttph_epu32 {:016x}\n", unsafe { mm_mask_cvttph_epu32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttph_epu32 {:016x}\n", unsafe { mm256_mask_cvttph_epu32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvttph_epu32 {:016x}\n", unsafe { mm512_mask_cvttph_epu32() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_cvttsh_u32 {:016x}\n", unsafe { mm_cvttsh_u32() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtph_epi64 {:016x}\n", unsafe { mm_mask_cvtph_epi64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtph_epi64 {:016x}\n", unsafe { mm256_mask_cvtph_epi64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtph_epi64 {:016x}\n", unsafe { mm512_mask_cvtph_epi64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtph_epu64 {:016x}\n", unsafe { mm_mask_cvtph_epu64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtph_epu64 {:016x}\n", unsafe { mm256_mask_cvtph_epu64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtph_epu64 {:016x}\n", unsafe { mm512_mask_cvtph_epu64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttph_epi64 {:016x}\n", unsafe { mm_mask_cvttph_epi64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttph_epi64 {:016x}\n", unsafe { mm256_mask_cvttph_epi64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvttph_epi64 {:016x}\n", unsafe { mm512_mask_cvttph_epi64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvttph_epu64 {:016x}\n", unsafe { mm_mask_cvttph_epu64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvttph_epu64 {:016x}\n", unsafe { mm256_mask_cvttph_epu64() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvttph_epu64 {:016x}\n", unsafe { mm512_mask_cvttph_epu64() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtxph_ps {:016x}\n", unsafe { mm_mask_cvtxph_ps() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtxph_ps {:016x}\n", unsafe { mm256_mask_cvtxph_ps() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtxph_ps {:016x}\n", unsafe { mm512_mask_cvtxph_ps() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_cvtsh_ss {:016x}\n", unsafe { mm_mask_cvtsh_ss() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtph_pd {:016x}\n", unsafe { mm_mask_cvtph_pd() }); }
    if is_x86_feature_detected!("avx512fp16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_mask_cvtph_pd {:016x}\n", unsafe { mm256_mask_cvtph_pd() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm512_mask_cvtph_pd {:016x}\n", unsafe { mm512_mask_cvtph_pd() }); }
    if is_x86_feature_detected!("avx512fp16") { out += &format!("_mm_mask_cvtsh_sd {:016x}\n", unsafe { mm_mask_cvtsh_sd() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm_bcstnebf16_ps {:016x}\n", unsafe { mm_bcstnebf16_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm256_bcstnebf16_ps {:016x}\n", unsafe { mm256_bcstnebf16_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm_bcstnesh_ps {:016x}\n", unsafe { mm_bcstnesh_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm256_bcstnesh_ps {:016x}\n", unsafe { mm256_bcstnesh_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm_cvtneebf16_ps {:016x}\n", unsafe { mm_cvtneebf16_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm256_cvtneebf16_ps {:016x}\n", unsafe { mm256_cvtneebf16_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm_cvtneeph_ps {:016x}\n", unsafe { mm_cvtneeph_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm256_cvtneeph_ps {:016x}\n", unsafe { mm256_cvtneeph_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm_cvtneobf16_ps {:016x}\n", unsafe { mm_cvtneobf16_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm256_cvtneobf16_ps {:016x}\n", unsafe { mm256_cvtneobf16_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm_cvtneoph_ps {:016x}\n", unsafe { mm_cvtneoph_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm256_cvtneoph_ps {:016x}\n", unsafe { mm256_cvtneoph_ps() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm_cvtneps_avx_pbh {:016x}\n", unsafe { mm_cvtneps_avx_pbh() }); }
    if is_x86_feature_detected!("avxneconvert") { out += &format!("_mm256_cvtneps_avx_pbh {:016x}\n", unsafe { mm256_cvtneps_avx_pbh() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_cvtne2ps_pbh {:016x}\n", unsafe { mm_cvtne2ps_pbh() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtne2ps_pbh {:016x}\n", unsafe { mm256_cvtne2ps_pbh() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtne2ps_pbh {:016x}\n", unsafe { mm512_cvtne2ps_pbh() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_cvtneps_pbh {:016x}\n", unsafe { mm256_cvtneps_pbh() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512f") { out += &format!("_mm512_cvtneps_pbh {:016x}\n", unsafe { mm512_cvtneps_pbh() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_dpbf16_ps {:016x}\n", unsafe { mm_dpbf16_ps() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm256_dpbf16_ps {:016x}\n", unsafe { mm256_dpbf16_ps() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512f") { out += &format!("_mm512_dpbf16_ps {:016x}\n", unsafe { mm512_dpbf16_ps() }); }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512vl") { out += &format!("_mm_mask_cvtneps_pbh {:016x}\n", unsafe { mm_mask_cvtneps_pbh() }); }
    if is_x86_feature_detected!("f16c") { out += &format!("_mm_cvtph_ps {:016x}\n", unsafe { mm_cvtph_ps() }); }
    if is_x86_feature_detected!("f16c") { out += &format!("_mm256_cvtph_ps {:016x}\n", unsafe { mm256_cvtph_ps() }); }
    out
}
