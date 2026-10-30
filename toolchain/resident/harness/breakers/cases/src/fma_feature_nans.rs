//! `_mm_fmadd_ps` and `_mm_fmadd_sd`, in a function with FMA, are LLVM's `vfmadd213ps` and
//! `vfmadd213sd` with the factors the other way round from libm's `fmaf` and `fma`, which
//! ra-jit called lane by lane: of two NaN factors rustc's kept the first and ra-jit's the
//! second. At `-O0` LLVM also folds the negations around such a call into it, or not, by the
//! code after it in its block (`_mm_fnmadd_ss` is one `vfnmadd213ss`, `_mm_fmsub_ps` a
//! `vpxor` and a `vfmadd213ps`), and a negation it folds keeps a NaN's sign where one it does
//! not flips it. Each FMA wrapper of `f32` and `f64` here, on lanes of NaNs of either sign,
//! quiet and signaling, and of numbers, under masks of every kind, hashed. In a breaker the
//! wrappers are rustc's, the crate's instances of them; `allrajit.py` builds this as a program
//! of its own, the standard library ra-jit's, where they are ra-jit's.
#![cfg_attr(not(target_arch = "x86_64"), allow(dead_code))]

#[cfg(target_arch = "x86_64")]
pub fn run() -> Vec<(&'static str, u64)> {
    if !(is_x86_feature_detected!("fma")
        && is_x86_feature_detected!("avx512f")
        && is_x86_feature_detected!("avx512vl"))
    {
        return Vec::new();
    }
    unsafe { fused() }
}

const ROUNDS: u64 = 64;

/// Which of the ten lanes of the pool, by a hash of the round, the operand and the lane.
fn pick(round: u64, operand: u64, lane: u64) -> usize {
    let mut z = (round << 16 | operand << 8 | lane).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z ^ (z >> 31)) % 10) as usize
}

fn lanes_f32<const N: usize>(round: u64, operand: u64) -> [u32; N] {
    let pool = [
        0x7fc0_0001 + operand as u32,
        0xffc0_0011 + operand as u32,
        0x7f80_0021 + operand as u32,
        0xff80_0031 + operand as u32,
        0x3fc0_0000,
        0xc000_0000,
        0x3e80_0000,
        0x7f80_0000,
        0x8000_0000,
        0x4040_0000,
    ];
    std::array::from_fn(|i| pool[pick(round, operand, i as u64)])
}

fn lanes_f64<const N: usize>(round: u64, operand: u64) -> [u64; N] {
    let pool = [
        0x7ff8_0000_0000_0001 + operand,
        0xfff8_0000_0000_0011 + operand,
        0x7ff0_0000_0000_0021 + operand,
        0xfff0_0000_0000_0031 + operand,
        0x3ff8_0000_0000_0000,
        0xc000_0000_0000_0000,
        0x3fd0_0000_0000_0000,
        0x7ff0_0000_0000_0000,
        0x8000_0000_0000_0000,
        0x4008_0000_0000_0000,
    ];
    std::array::from_fn(|i| pool[pick(round, operand, i as u64)])
}

fn mask(round: u64) -> u64 {
    match round % 4 {
        0 => !0,
        1 => 0,
        2 => 0x5555_5555_5555_5555 >> (round % 3),
        _ => (round + 1).wrapping_mul(0x2545_f491_4f6c_dd1d) >> 7,
    }
}

/// An operand of a wrapper, made for a round, and a result's bits.
trait Arg: Sized {
    fn make(round: u64, operand: u64) -> Self;
    fn bits(self) -> Vec<u64>;
}

macro_rules! vectors {
    ($($ty:ident: $lanes:ident, $bits:ty, $n:literal;)*) => {$(
        #[cfg(target_arch = "x86_64")]
        impl Arg for std::arch::x86_64::$ty {
            fn make(round: u64, operand: u64) -> Self {
                unsafe { std::mem::transmute($lanes::<$n>(round, operand)) }
            }
            fn bits(self) -> Vec<u64> {
                unsafe { std::mem::transmute::<Self, [$bits; $n]>(self) }.map(u64::from).to_vec()
            }
        }
    )*};
}

vectors! {
    __m128: lanes_f32, u32, 4;
    __m256: lanes_f32, u32, 8;
    __m512: lanes_f32, u32, 16;
    __m128d: lanes_f64, u64, 2;
    __m256d: lanes_f64, u64, 4;
    __m512d: lanes_f64, u64, 8;
}

macro_rules! masks {
    ($($ty:ty),*) => {$(
        impl Arg for $ty {
            fn make(round: u64, _: u64) -> Self {
                mask(round) as $ty
            }
            fn bits(self) -> Vec<u64> {
                vec![self.into()]
            }
        }
    )*};
}

masks!(u8, u16);

fn hash(h: u64, lanes: &[u64]) -> u64 {
    lanes.iter().fold(h, |h, &it| (h ^ it).wrapping_mul(0x0100_0000_01b3).rotate_left(17))
}

/// Each wrapper, by the positions of its operands, on each round.
macro_rules! rows {
    ($($name:ident($($at:literal),*),)*) => {
        vec![$((
            stringify!($name),
            (0..ROUNDS).fold(0, |h, round| hash(h, &$name($(Arg::make(round, $at)),*).bits())),
        )),*]
    };
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "fma,avx512f,avx512vl")]
unsafe fn fused() -> Vec<(&'static str, u64)> {
    use std::arch::x86_64::*;
    rows! {
        _mm_fmadd_pd(0, 1, 2),
        _mm256_fmadd_pd(0, 1, 2),
        _mm_fmadd_ps(0, 1, 2),
        _mm256_fmadd_ps(0, 1, 2),
        _mm_fmadd_sd(0, 1, 2),
        _mm_fmadd_ss(0, 1, 2),
        _mm_fmaddsub_pd(0, 1, 2),
        _mm256_fmaddsub_pd(0, 1, 2),
        _mm_fmaddsub_ps(0, 1, 2),
        _mm256_fmaddsub_ps(0, 1, 2),
        _mm_fmsub_pd(0, 1, 2),
        _mm256_fmsub_pd(0, 1, 2),
        _mm_fmsub_ps(0, 1, 2),
        _mm256_fmsub_ps(0, 1, 2),
        _mm_fmsub_sd(0, 1, 2),
        _mm_fmsub_ss(0, 1, 2),
        _mm_fmsubadd_pd(0, 1, 2),
        _mm256_fmsubadd_pd(0, 1, 2),
        _mm_fmsubadd_ps(0, 1, 2),
        _mm256_fmsubadd_ps(0, 1, 2),
        _mm_fnmadd_pd(0, 1, 2),
        _mm256_fnmadd_pd(0, 1, 2),
        _mm_fnmadd_ps(0, 1, 2),
        _mm256_fnmadd_ps(0, 1, 2),
        _mm_fnmadd_sd(0, 1, 2),
        _mm_fnmadd_ss(0, 1, 2),
        _mm_fnmsub_pd(0, 1, 2),
        _mm256_fnmsub_pd(0, 1, 2),
        _mm_fnmsub_ps(0, 1, 2),
        _mm256_fnmsub_ps(0, 1, 2),
        _mm_fnmsub_sd(0, 1, 2),
        _mm_fnmsub_ss(0, 1, 2),
        _mm512_fmadd_ps(0, 1, 2),
        _mm512_mask_fmadd_ps(0, 1, 2, 3),
        _mm512_maskz_fmadd_ps(0, 1, 2, 3),
        _mm512_mask3_fmadd_ps(0, 1, 2, 3),
        _mm256_mask_fmadd_ps(0, 1, 2, 3),
        _mm256_maskz_fmadd_ps(0, 1, 2, 3),
        _mm256_mask3_fmadd_ps(0, 1, 2, 3),
        _mm_mask_fmadd_ps(0, 1, 2, 3),
        _mm_maskz_fmadd_ps(0, 1, 2, 3),
        _mm_mask3_fmadd_ps(0, 1, 2, 3),
        _mm512_fmadd_pd(0, 1, 2),
        _mm512_mask_fmadd_pd(0, 1, 2, 3),
        _mm512_maskz_fmadd_pd(0, 1, 2, 3),
        _mm512_mask3_fmadd_pd(0, 1, 2, 3),
        _mm256_mask_fmadd_pd(0, 1, 2, 3),
        _mm256_maskz_fmadd_pd(0, 1, 2, 3),
        _mm256_mask3_fmadd_pd(0, 1, 2, 3),
        _mm_mask_fmadd_pd(0, 1, 2, 3),
        _mm_maskz_fmadd_pd(0, 1, 2, 3),
        _mm_mask3_fmadd_pd(0, 1, 2, 3),
        _mm512_fmsub_ps(0, 1, 2),
        _mm512_mask_fmsub_ps(0, 1, 2, 3),
        _mm512_maskz_fmsub_ps(0, 1, 2, 3),
        _mm512_mask3_fmsub_ps(0, 1, 2, 3),
        _mm256_mask_fmsub_ps(0, 1, 2, 3),
        _mm256_maskz_fmsub_ps(0, 1, 2, 3),
        _mm256_mask3_fmsub_ps(0, 1, 2, 3),
        _mm_mask_fmsub_ps(0, 1, 2, 3),
        _mm_maskz_fmsub_ps(0, 1, 2, 3),
        _mm_mask3_fmsub_ps(0, 1, 2, 3),
        _mm512_fmsub_pd(0, 1, 2),
        _mm512_mask_fmsub_pd(0, 1, 2, 3),
        _mm512_maskz_fmsub_pd(0, 1, 2, 3),
        _mm512_mask3_fmsub_pd(0, 1, 2, 3),
        _mm256_mask_fmsub_pd(0, 1, 2, 3),
        _mm256_maskz_fmsub_pd(0, 1, 2, 3),
        _mm256_mask3_fmsub_pd(0, 1, 2, 3),
        _mm_mask_fmsub_pd(0, 1, 2, 3),
        _mm_maskz_fmsub_pd(0, 1, 2, 3),
        _mm_mask3_fmsub_pd(0, 1, 2, 3),
        _mm512_fmaddsub_ps(0, 1, 2),
        _mm512_mask_fmaddsub_ps(0, 1, 2, 3),
        _mm512_maskz_fmaddsub_ps(0, 1, 2, 3),
        _mm512_mask3_fmaddsub_ps(0, 1, 2, 3),
        _mm256_mask_fmaddsub_ps(0, 1, 2, 3),
        _mm256_maskz_fmaddsub_ps(0, 1, 2, 3),
        _mm256_mask3_fmaddsub_ps(0, 1, 2, 3),
        _mm_mask_fmaddsub_ps(0, 1, 2, 3),
        _mm_maskz_fmaddsub_ps(0, 1, 2, 3),
        _mm_mask3_fmaddsub_ps(0, 1, 2, 3),
        _mm512_fmaddsub_pd(0, 1, 2),
        _mm512_mask_fmaddsub_pd(0, 1, 2, 3),
        _mm512_maskz_fmaddsub_pd(0, 1, 2, 3),
        _mm512_mask3_fmaddsub_pd(0, 1, 2, 3),
        _mm256_mask_fmaddsub_pd(0, 1, 2, 3),
        _mm256_maskz_fmaddsub_pd(0, 1, 2, 3),
        _mm256_mask3_fmaddsub_pd(0, 1, 2, 3),
        _mm_mask_fmaddsub_pd(0, 1, 2, 3),
        _mm_maskz_fmaddsub_pd(0, 1, 2, 3),
        _mm_mask3_fmaddsub_pd(0, 1, 2, 3),
        _mm512_fmsubadd_ps(0, 1, 2),
        _mm512_mask_fmsubadd_ps(0, 1, 2, 3),
        _mm512_maskz_fmsubadd_ps(0, 1, 2, 3),
        _mm512_mask3_fmsubadd_ps(0, 1, 2, 3),
        _mm256_mask_fmsubadd_ps(0, 1, 2, 3),
        _mm256_maskz_fmsubadd_ps(0, 1, 2, 3),
        _mm256_mask3_fmsubadd_ps(0, 1, 2, 3),
        _mm_mask_fmsubadd_ps(0, 1, 2, 3),
        _mm_maskz_fmsubadd_ps(0, 1, 2, 3),
        _mm_mask3_fmsubadd_ps(0, 1, 2, 3),
        _mm512_fmsubadd_pd(0, 1, 2),
        _mm512_mask_fmsubadd_pd(0, 1, 2, 3),
        _mm512_maskz_fmsubadd_pd(0, 1, 2, 3),
        _mm512_mask3_fmsubadd_pd(0, 1, 2, 3),
        _mm256_mask_fmsubadd_pd(0, 1, 2, 3),
        _mm256_maskz_fmsubadd_pd(0, 1, 2, 3),
        _mm256_mask3_fmsubadd_pd(0, 1, 2, 3),
        _mm_mask_fmsubadd_pd(0, 1, 2, 3),
        _mm_maskz_fmsubadd_pd(0, 1, 2, 3),
        _mm_mask3_fmsubadd_pd(0, 1, 2, 3),
        _mm512_fnmadd_ps(0, 1, 2),
        _mm512_mask_fnmadd_ps(0, 1, 2, 3),
        _mm512_maskz_fnmadd_ps(0, 1, 2, 3),
        _mm512_mask3_fnmadd_ps(0, 1, 2, 3),
        _mm256_mask_fnmadd_ps(0, 1, 2, 3),
        _mm256_maskz_fnmadd_ps(0, 1, 2, 3),
        _mm256_mask3_fnmadd_ps(0, 1, 2, 3),
        _mm_mask_fnmadd_ps(0, 1, 2, 3),
        _mm_maskz_fnmadd_ps(0, 1, 2, 3),
        _mm_mask3_fnmadd_ps(0, 1, 2, 3),
        _mm512_fnmadd_pd(0, 1, 2),
        _mm512_mask_fnmadd_pd(0, 1, 2, 3),
        _mm512_maskz_fnmadd_pd(0, 1, 2, 3),
        _mm512_mask3_fnmadd_pd(0, 1, 2, 3),
        _mm256_mask_fnmadd_pd(0, 1, 2, 3),
        _mm256_maskz_fnmadd_pd(0, 1, 2, 3),
        _mm256_mask3_fnmadd_pd(0, 1, 2, 3),
        _mm_mask_fnmadd_pd(0, 1, 2, 3),
        _mm_maskz_fnmadd_pd(0, 1, 2, 3),
        _mm_mask3_fnmadd_pd(0, 1, 2, 3),
        _mm512_fnmsub_ps(0, 1, 2),
        _mm512_mask_fnmsub_ps(0, 1, 2, 3),
        _mm512_maskz_fnmsub_ps(0, 1, 2, 3),
        _mm512_mask3_fnmsub_ps(0, 1, 2, 3),
        _mm256_mask_fnmsub_ps(0, 1, 2, 3),
        _mm256_maskz_fnmsub_ps(0, 1, 2, 3),
        _mm256_mask3_fnmsub_ps(0, 1, 2, 3),
        _mm_mask_fnmsub_ps(0, 1, 2, 3),
        _mm_maskz_fnmsub_ps(0, 1, 2, 3),
        _mm_mask3_fnmsub_ps(0, 1, 2, 3),
        _mm512_fnmsub_pd(0, 1, 2),
        _mm512_mask_fnmsub_pd(0, 1, 2, 3),
        _mm512_maskz_fnmsub_pd(0, 1, 2, 3),
        _mm512_mask3_fnmsub_pd(0, 1, 2, 3),
        _mm256_mask_fnmsub_pd(0, 1, 2, 3),
        _mm256_maskz_fnmsub_pd(0, 1, 2, 3),
        _mm256_mask3_fnmsub_pd(0, 1, 2, 3),
        _mm_mask_fnmsub_pd(0, 1, 2, 3),
        _mm_maskz_fnmsub_pd(0, 1, 2, 3),
        _mm_mask3_fnmsub_pd(0, 1, 2, 3),
        _mm_mask_fmadd_ss(0, 1, 2, 3),
        _mm_maskz_fmadd_ss(0, 1, 2, 3),
        _mm_mask3_fmadd_ss(0, 1, 2, 3),
        _mm_mask_fmadd_sd(0, 1, 2, 3),
        _mm_maskz_fmadd_sd(0, 1, 2, 3),
        _mm_mask3_fmadd_sd(0, 1, 2, 3),
        _mm_mask_fmsub_ss(0, 1, 2, 3),
        _mm_maskz_fmsub_ss(0, 1, 2, 3),
        _mm_mask3_fmsub_ss(0, 1, 2, 3),
        _mm_mask_fmsub_sd(0, 1, 2, 3),
        _mm_maskz_fmsub_sd(0, 1, 2, 3),
        _mm_mask3_fmsub_sd(0, 1, 2, 3),
        _mm_mask_fnmadd_ss(0, 1, 2, 3),
        _mm_maskz_fnmadd_ss(0, 1, 2, 3),
        _mm_mask3_fnmadd_ss(0, 1, 2, 3),
        _mm_mask_fnmadd_sd(0, 1, 2, 3),
        _mm_maskz_fnmadd_sd(0, 1, 2, 3),
        _mm_mask3_fnmadd_sd(0, 1, 2, 3),
        _mm_mask_fnmsub_ss(0, 1, 2, 3),
        _mm_maskz_fnmsub_ss(0, 1, 2, 3),
        _mm_mask3_fnmsub_ss(0, 1, 2, 3),
        _mm_mask_fnmsub_sd(0, 1, 2, 3),
        _mm_maskz_fnmsub_sd(0, 1, 2, 3),
        _mm_mask3_fnmsub_sd(0, 1, 2, 3),
    }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> Vec<(&'static str, u64)> {
    Vec::new()
}
