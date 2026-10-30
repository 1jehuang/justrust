//! What `core::arch`'s F16C and AVX512-BF16 functions do, done here so that ra-jit compiles it:
//! the `simd_cast` of lanes of f16 `_mm_cvtph_ps` and `_mm256_cvtph_ps` make, and the LLVM
//! intrinsics `avx512bf16.rs` declares, with its types.

use core::intrinsics::simd::simd_cast;
use core::mem::transmute;
use core::simd::Simd;

type F32x4 = Simd<f32, 4>;
type F32x8 = Simd<f32, 8>;
type F32x16 = Simd<f32, 16>;
type I16x8 = Simd<i16, 8>;
type I16x16 = Simd<i16, 16>;
type I16x32 = Simd<i16, 32>;

#[allow(improper_ctypes)]
unsafe extern "llvm-intrinsic" {
    #[link_name = "llvm.x86.avx512bf16.cvtne2ps2bf16.128"]
    fn cvtne2ps2bf16(a: F32x4, b: F32x4) -> I16x8;
    #[link_name = "llvm.x86.avx512bf16.cvtne2ps2bf16.256"]
    fn cvtne2ps2bf16_256(a: F32x8, b: F32x8) -> I16x16;
    #[link_name = "llvm.x86.avx512bf16.cvtne2ps2bf16.512"]
    fn cvtne2ps2bf16_512(a: F32x16, b: F32x16) -> I16x32;
    #[link_name = "llvm.x86.avx512bf16.mask.cvtneps2bf16.128"]
    fn cvtneps2bf16_128(a: F32x4, src: I16x8, k: u8) -> I16x8;
    #[link_name = "llvm.x86.avx512bf16.cvtneps2bf16.256"]
    fn cvtneps2bf16_256(a: F32x8) -> I16x8;
    #[link_name = "llvm.x86.avx512bf16.cvtneps2bf16.512"]
    fn cvtneps2bf16_512(a: F32x16) -> I16x16;
    #[link_name = "llvm.x86.avx512bf16.dpbf16ps.128"]
    fn dpbf16ps(a: F32x4, b: I16x8, c: I16x8) -> F32x4;
    #[link_name = "llvm.x86.avx512bf16.dpbf16ps.256"]
    fn dpbf16ps_256(a: F32x8, b: I16x16, c: I16x16) -> F32x8;
    #[link_name = "llvm.x86.avx512bf16.dpbf16ps.512"]
    fn dpbf16ps_512(a: F32x16, b: I16x32, c: I16x32) -> F32x16;
}

const HALVES: [[u16; 8]; 3] = [
    [0x3c00, 0xc500, 0x7c01, 0xfe01, 0x0001, 0x8000, 0x7bff, 0x7c00],
    [0x3555, 0x83ff, 0x0400, 0xfc00, 0x7e00, 0x7d55, 0x5640, 0xbbff],
    [0x0000, 0xffff, 0x8001, 0x3c01, 0x4248, 0x7a00, 0x03ff, 0xc000],
];

const SINGLES: [[u32; 16]; 2] = [
    [
        0x3f80_8000, 0x3f81_8000, 0x7f80_0001, 0xffc0_0001, 0x0000_0001, 0x8000_0000, 0x7f7f_ffff, 0x3fc0_0000,
        0xc010_0000, 0x4049_0fdb, 0x7f80_0000, 0xff80_0000, 0x3f80_0001, 0x3f80_ffff, 0x0080_0000, 0x807f_ffff,
    ],
    [
        0x4000_0000, 0xbf80_0000, 0x3eaa_aaab, 0x7fc0_0000, 0x42f6_e979, 0x3f7f_ffff, 0x0000_0000, 0xc2c8_0000,
        0x3f8c_0000, 0x7f80_0001, 0x0040_0000, 0xbf80_8000, 0x4120_0000, 0x3dcc_cccd, 0x477f_e000, 0xc049_0fdb,
    ],
];

const MASKS: [u8; 3] = [0, 0x5a, 0xff];

#[target_feature(enable = "f16c")]
unsafe fn widen(bits: [u16; 8]) -> Vec<u32> {
    let four: F32x4 = simd_cast(transmute::<[u16; 4], Simd<f16, 4>>([bits[0], bits[1], bits[2], bits[3]]));
    let eight: F32x8 = simd_cast(transmute::<[u16; 8], Simd<f16, 8>>(bits));
    let mut out: Vec<u32> = transmute::<F32x4, [u32; 4]>(four).to_vec();
    out.extend(transmute::<F32x8, [u32; 8]>(eight));
    out
}

fn singles<const N: usize>(bits: &[u32; 16]) -> Simd<f32, N> {
    Simd::from_slice(&bits.map(f32::from_bits)[..N])
}

#[target_feature(enable = "avx512bf16,avx512vl")]
unsafe fn brains(a: &[u32; 16], b: &[u32; 16], k: u8) -> Vec<u32> {
    let mut out = Vec::new();
    let two4 = cvtne2ps2bf16(singles(a), singles(b));
    let two8 = cvtne2ps2bf16_256(singles(a), singles(b));
    let two16 = cvtne2ps2bf16_512(singles(a), singles(b));
    let src = Simd::from_array([0x1234i16, -2, 0x7fc1, 3, 4, 5, 6, 7]);
    let one4 = cvtneps2bf16_128(singles(b), src, k);
    let one8 = cvtneps2bf16_256(singles(a));
    let one16 = cvtneps2bf16_512(singles(b));
    for lane in two4.to_array().into_iter().chain(two8.to_array()).chain(two16.to_array()) {
        out.push(lane as u16 as u32);
    }
    for lane in one4.to_array().into_iter().chain(one8.to_array()).chain(one16.to_array()) {
        out.push(lane as u16 as u32);
    }
    let dot4 = dpbf16ps(singles(b), two4, cvtne2ps2bf16(singles(b), singles(a)));
    let dot8 = dpbf16ps_256(singles(a), two8, two8);
    let dot16 = dpbf16ps_512(singles(b), two16, cvtne2ps2bf16_512(singles(b), singles(a)));
    for lane in dot4.to_array().into_iter().chain(dot8.to_array()).chain(dot16.to_array()) {
        out.push(lane.to_bits());
    }
    out
}

pub fn run() -> String {
    let mut out = String::new();
    if is_x86_feature_detected!("f16c") {
        for bits in HALVES {
            out += &format!("widen {bits:x?} {:x?}\n", unsafe { widen(bits) });
        }
    }
    if is_x86_feature_detected!("avx512bf16") && is_x86_feature_detected!("avx512vl") {
        for (a, b) in [(&SINGLES[0], &SINGLES[1]), (&SINGLES[1], &SINGLES[0])] {
            for k in MASKS {
                out += &format!("brains {k:x} {:x?}\n", unsafe { brains(a, b, k) });
            }
        }
    }
    out
}
