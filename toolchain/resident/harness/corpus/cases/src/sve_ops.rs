//! Generated: each function of `core::arch::aarch64`'s SVE and SVE2 whose LLVM intrinsics
//! ra-jit has a stub of, called with fixed inputs (edge values, partial predicates, each end
//! of an immediate's range), each result hashed over the whole vector length. A machine
//! without one of the extensions prints that it has not.

use core::arch::aarch64::*;

/// Inputs: bytes, words and floats of edge values, read as any vector or through pointers.
pub struct In {
    bytes: Vec<u8>,
    f32s: Vec<f32>,
    f64s: Vec<f64>,
}

#[allow(dead_code, reason = "a reader of every vector type; not all are called")]
impl In {
    fn new() -> In {
        let mut state = 0x9e37_79b9_7f4a_7c15u64;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut bytes: Vec<u8> = (0..8192).map(|_| next() as u8).collect();
        for (at, edge) in [0u8, 0xff, 0x80, 0x7f, 1].iter().enumerate() {
            for it in (at * 13..8192).step_by(61) {
                bytes[it] = *edge;
            }
        }
        let floats =
            [0.0, -0.0, 1.0, -1.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e-310, 3.0e38, 0.5];
        let mut f64s: Vec<f64> = (0..1024).map(|_| (next() as i64 as f64) / 1e15).collect();
        let mut f32s: Vec<f32> = (0..2048).map(|_| (next() as i32 as f32) / 1e6).collect();
        for at in (0..1024).step_by(3) {
            f64s[at] = floats[at % floats.len()];
        }
        for at in (0..2048).step_by(3) {
            f32s[at] = floats[at % floats.len()] as f32;
        }
        In { bytes, f32s, f64s }
    }

    /// The address of the `at`th element of the bytes as `T`.
    fn at<T>(&self, at: usize) -> *const T {
        self.bytes[at * core::mem::size_of::<T>()..].as_ptr().cast()
    }
}

/// The hash of each result so far, and memory stores write to.
pub struct Out {
    text: String,
    memory: Vec<u8>,
}

#[allow(dead_code, reason = "a writer of every vector type; not all are called")]
impl Out {
    fn at<T>(&mut self, at: usize) -> *mut T {
        self.memory[at * core::mem::size_of::<T>()..].as_mut_ptr().cast()
    }

    fn bytes(&mut self, bytes: &[u8]) {
        let mut hash = 0x811c_9dc5u32;
        for byte in bytes {
            hash = (hash ^ u32::from(*byte)).wrapping_mul(0x0100_0193);
        }
        self.text.push_str(&format!("{:04x}", hash as u16 ^ (hash >> 16) as u16));
    }

    fn scalar<T: Bits>(&mut self, value: T) {
        self.bytes(&value.bits().to_le_bytes());
    }

    /// What the stores wrote, then the memory as before.
    fn written(&mut self) {
        let memory = core::mem::take(&mut self.memory);
        self.bytes(&memory);
        self.memory = vec![0x5a; memory.len()];
    }
}

trait Bits {
    fn bits(self) -> u64;
}

macro_rules! bits {
    ($($t:ty),*) => {$(impl Bits for $t { fn bits(self) -> u64 { self as u64 } })*};
}
bits!(i8, u8, i16, u16, i32, u32, i64, u64, bool);
impl Bits for f32 {
    fn bits(self) -> u64 {
        u64::from(self.to_bits())
    }
}
impl Bits for f64 {
    fn bits(self) -> u64 {
        self.to_bits()
    }
}

#[allow(dead_code, reason = "a reader of every vector type; not all are called")]
impl In {
    /// A predicate: all lanes, the first five, and two of bits of the inputs.
    #[target_feature(enable = "sve")]
    unsafe fn p(&self, k: usize) -> svbool_t {
        match k {
            0 => svptrue_b8(),
            1 => svwhilelt_b8_s32(0, 5),
            // SAFETY: a vector of bytes from 512 * 3 on is in the inputs.
            _ => svcmpne_n_u8(
                svptrue_b8(),
                unsafe { svld1_u8(svptrue_b8(), self.at::<u8>(512 * k)) },
                0x80,
            ),
        }
    }

    /// Offsets or indices of lanes of a gather or scatter: from 0 to 63, or from -32 to 31.
    #[target_feature(enable = "sve")]
    unsafe fn ou64(&self, k: usize) -> svuint64_t {
        unsafe { svand_n_u64_x(svptrue_b64(), self.u64(k), 63) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn os64(&self, k: usize) -> svint64_t {
        unsafe { svsub_n_s64_x(svptrue_b64(), svand_n_s64_x(svptrue_b64(), self.s64(k), 63), 32) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn ou32(&self, k: usize) -> svuint32_t {
        unsafe { svand_n_u32_x(svptrue_b32(), self.u32(k), 63) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn os32(&self, k: usize) -> svint32_t {
        unsafe { svsub_n_s32_x(svptrue_b32(), svand_n_s32_x(svptrue_b32(), self.s32(k), 63), 32) }
    }
    /// Addresses of lanes from `at` to 511 bytes past it.
    #[target_feature(enable = "sve")]
    unsafe fn bu64(&self, k: usize, at: *const u8) -> svuint64_t {
        unsafe {
            svadd_n_u64_x(svptrue_b64(), svand_n_u64_x(svptrue_b64(), self.u64(k), 511), at as u64)
        }
    }
    /// Offsets of lanes from a scalar address, up to 255 bytes, as the 32-bit addresses
    /// of a gather or scatter are.
    #[target_feature(enable = "sve")]
    unsafe fn bu32(&self, k: usize) -> svuint32_t {
        unsafe { svand_n_u32_x(svptrue_b32(), self.u32(k), 255) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s8(&self, k: usize) -> svint8_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_s8(svptrue_b8(), self.at::<i8>(k * 1024 / 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s8x2(&self, k: usize) -> svint8x2_t {
        unsafe { svcreate2_s8(self.s8(k), self.s8(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s8x3(&self, k: usize) -> svint8x3_t {
        unsafe { svcreate3_s8(self.s8(k), self.s8(k + 1), self.s8(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s8x4(&self, k: usize) -> svint8x4_t {
        unsafe { svcreate4_s8(self.s8(k), self.s8(k + 1), self.s8(k + 2), self.s8(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u8(&self, k: usize) -> svuint8_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_u8(svptrue_b8(), self.at::<u8>(k * 1024 / 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u8x2(&self, k: usize) -> svuint8x2_t {
        unsafe { svcreate2_u8(self.u8(k), self.u8(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u8x3(&self, k: usize) -> svuint8x3_t {
        unsafe { svcreate3_u8(self.u8(k), self.u8(k + 1), self.u8(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u8x4(&self, k: usize) -> svuint8x4_t {
        unsafe { svcreate4_u8(self.u8(k), self.u8(k + 1), self.u8(k + 2), self.u8(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s16(&self, k: usize) -> svint16_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_s16(svptrue_b16(), self.at::<i16>(k * 1024 / 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s16x2(&self, k: usize) -> svint16x2_t {
        unsafe { svcreate2_s16(self.s16(k), self.s16(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s16x3(&self, k: usize) -> svint16x3_t {
        unsafe { svcreate3_s16(self.s16(k), self.s16(k + 1), self.s16(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s16x4(&self, k: usize) -> svint16x4_t {
        unsafe { svcreate4_s16(self.s16(k), self.s16(k + 1), self.s16(k + 2), self.s16(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u16(&self, k: usize) -> svuint16_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_u16(svptrue_b16(), self.at::<u16>(k * 1024 / 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u16x2(&self, k: usize) -> svuint16x2_t {
        unsafe { svcreate2_u16(self.u16(k), self.u16(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u16x3(&self, k: usize) -> svuint16x3_t {
        unsafe { svcreate3_u16(self.u16(k), self.u16(k + 1), self.u16(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u16x4(&self, k: usize) -> svuint16x4_t {
        unsafe { svcreate4_u16(self.u16(k), self.u16(k + 1), self.u16(k + 2), self.u16(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s32(&self, k: usize) -> svint32_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_s32(svptrue_b32(), self.at::<i32>(k * 1024 / 4)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s32x2(&self, k: usize) -> svint32x2_t {
        unsafe { svcreate2_s32(self.s32(k), self.s32(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s32x3(&self, k: usize) -> svint32x3_t {
        unsafe { svcreate3_s32(self.s32(k), self.s32(k + 1), self.s32(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s32x4(&self, k: usize) -> svint32x4_t {
        unsafe { svcreate4_s32(self.s32(k), self.s32(k + 1), self.s32(k + 2), self.s32(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u32(&self, k: usize) -> svuint32_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_u32(svptrue_b32(), self.at::<u32>(k * 1024 / 4)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u32x2(&self, k: usize) -> svuint32x2_t {
        unsafe { svcreate2_u32(self.u32(k), self.u32(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u32x3(&self, k: usize) -> svuint32x3_t {
        unsafe { svcreate3_u32(self.u32(k), self.u32(k + 1), self.u32(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u32x4(&self, k: usize) -> svuint32x4_t {
        unsafe { svcreate4_u32(self.u32(k), self.u32(k + 1), self.u32(k + 2), self.u32(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s64(&self, k: usize) -> svint64_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_s64(svptrue_b64(), self.at::<i64>(k * 1024 / 8)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s64x2(&self, k: usize) -> svint64x2_t {
        unsafe { svcreate2_s64(self.s64(k), self.s64(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s64x3(&self, k: usize) -> svint64x3_t {
        unsafe { svcreate3_s64(self.s64(k), self.s64(k + 1), self.s64(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s64x4(&self, k: usize) -> svint64x4_t {
        unsafe { svcreate4_s64(self.s64(k), self.s64(k + 1), self.s64(k + 2), self.s64(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u64(&self, k: usize) -> svuint64_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_u64(svptrue_b64(), self.at::<u64>(k * 1024 / 8)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u64x2(&self, k: usize) -> svuint64x2_t {
        unsafe { svcreate2_u64(self.u64(k), self.u64(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u64x3(&self, k: usize) -> svuint64x3_t {
        unsafe { svcreate3_u64(self.u64(k), self.u64(k + 1), self.u64(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u64x4(&self, k: usize) -> svuint64x4_t {
        unsafe { svcreate4_u64(self.u64(k), self.u64(k + 1), self.u64(k + 2), self.u64(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn f32(&self, k: usize) -> svfloat32_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_f32(svptrue_b32(), self.f32s[k * 67..].as_ptr()) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f32x2(&self, k: usize) -> svfloat32x2_t {
        unsafe { svcreate2_f32(self.f32(k), self.f32(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f32x3(&self, k: usize) -> svfloat32x3_t {
        unsafe { svcreate3_f32(self.f32(k), self.f32(k + 1), self.f32(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f32x4(&self, k: usize) -> svfloat32x4_t {
        unsafe { svcreate4_f32(self.f32(k), self.f32(k + 1), self.f32(k + 2), self.f32(k + 3)) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn f64(&self, k: usize) -> svfloat64_t {
        // SAFETY: four vectors of the largest length from there on are in the inputs.
        unsafe { svld1_f64(svptrue_b64(), self.f64s[k * 67..].as_ptr()) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f64x2(&self, k: usize) -> svfloat64x2_t {
        unsafe { svcreate2_f64(self.f64(k), self.f64(k + 1)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f64x3(&self, k: usize) -> svfloat64x3_t {
        unsafe { svcreate3_f64(self.f64(k), self.f64(k + 1), self.f64(k + 2)) }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f64x4(&self, k: usize) -> svfloat64x4_t {
        unsafe { svcreate4_f64(self.f64(k), self.f64(k + 1), self.f64(k + 2), self.f64(k + 3)) }
    }
}

#[allow(dead_code, reason = "a writer of every vector type; not all are called")]
impl Out {
    /// A predicate as a vector of a byte 1 for each lane set, else 0.
    #[target_feature(enable = "sve")]
    unsafe fn p(&mut self, p: svbool_t) {
        unsafe { self.u8(svsel_u8(p, svdup_n_u8(1), svdup_n_u8(0))) }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s8(&mut self, v: svint8_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_s8(svptrue_b8(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn s8x2(&mut self, v: svint8x2_t) {
        unsafe {
            self.s8(svget2_s8::<0>(v));
            self.s8(svget2_s8::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s8x3(&mut self, v: svint8x3_t) {
        unsafe {
            self.s8(svget3_s8::<0>(v));
            self.s8(svget3_s8::<1>(v));
            self.s8(svget3_s8::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s8x4(&mut self, v: svint8x4_t) {
        unsafe {
            self.s8(svget4_s8::<0>(v));
            self.s8(svget4_s8::<1>(v));
            self.s8(svget4_s8::<2>(v));
            self.s8(svget4_s8::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u8(&mut self, v: svuint8_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_u8(svptrue_b8(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn u8x2(&mut self, v: svuint8x2_t) {
        unsafe {
            self.u8(svget2_u8::<0>(v));
            self.u8(svget2_u8::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u8x3(&mut self, v: svuint8x3_t) {
        unsafe {
            self.u8(svget3_u8::<0>(v));
            self.u8(svget3_u8::<1>(v));
            self.u8(svget3_u8::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u8x4(&mut self, v: svuint8x4_t) {
        unsafe {
            self.u8(svget4_u8::<0>(v));
            self.u8(svget4_u8::<1>(v));
            self.u8(svget4_u8::<2>(v));
            self.u8(svget4_u8::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s16(&mut self, v: svint16_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_s16(svptrue_b16(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn s16x2(&mut self, v: svint16x2_t) {
        unsafe {
            self.s16(svget2_s16::<0>(v));
            self.s16(svget2_s16::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s16x3(&mut self, v: svint16x3_t) {
        unsafe {
            self.s16(svget3_s16::<0>(v));
            self.s16(svget3_s16::<1>(v));
            self.s16(svget3_s16::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s16x4(&mut self, v: svint16x4_t) {
        unsafe {
            self.s16(svget4_s16::<0>(v));
            self.s16(svget4_s16::<1>(v));
            self.s16(svget4_s16::<2>(v));
            self.s16(svget4_s16::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u16(&mut self, v: svuint16_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_u16(svptrue_b16(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn u16x2(&mut self, v: svuint16x2_t) {
        unsafe {
            self.u16(svget2_u16::<0>(v));
            self.u16(svget2_u16::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u16x3(&mut self, v: svuint16x3_t) {
        unsafe {
            self.u16(svget3_u16::<0>(v));
            self.u16(svget3_u16::<1>(v));
            self.u16(svget3_u16::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u16x4(&mut self, v: svuint16x4_t) {
        unsafe {
            self.u16(svget4_u16::<0>(v));
            self.u16(svget4_u16::<1>(v));
            self.u16(svget4_u16::<2>(v));
            self.u16(svget4_u16::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s32(&mut self, v: svint32_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_s32(svptrue_b32(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn s32x2(&mut self, v: svint32x2_t) {
        unsafe {
            self.s32(svget2_s32::<0>(v));
            self.s32(svget2_s32::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s32x3(&mut self, v: svint32x3_t) {
        unsafe {
            self.s32(svget3_s32::<0>(v));
            self.s32(svget3_s32::<1>(v));
            self.s32(svget3_s32::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s32x4(&mut self, v: svint32x4_t) {
        unsafe {
            self.s32(svget4_s32::<0>(v));
            self.s32(svget4_s32::<1>(v));
            self.s32(svget4_s32::<2>(v));
            self.s32(svget4_s32::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u32(&mut self, v: svuint32_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_u32(svptrue_b32(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn u32x2(&mut self, v: svuint32x2_t) {
        unsafe {
            self.u32(svget2_u32::<0>(v));
            self.u32(svget2_u32::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u32x3(&mut self, v: svuint32x3_t) {
        unsafe {
            self.u32(svget3_u32::<0>(v));
            self.u32(svget3_u32::<1>(v));
            self.u32(svget3_u32::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u32x4(&mut self, v: svuint32x4_t) {
        unsafe {
            self.u32(svget4_u32::<0>(v));
            self.u32(svget4_u32::<1>(v));
            self.u32(svget4_u32::<2>(v));
            self.u32(svget4_u32::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn s64(&mut self, v: svint64_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_s64(svptrue_b64(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn s64x2(&mut self, v: svint64x2_t) {
        unsafe {
            self.s64(svget2_s64::<0>(v));
            self.s64(svget2_s64::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s64x3(&mut self, v: svint64x3_t) {
        unsafe {
            self.s64(svget3_s64::<0>(v));
            self.s64(svget3_s64::<1>(v));
            self.s64(svget3_s64::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn s64x4(&mut self, v: svint64x4_t) {
        unsafe {
            self.s64(svget4_s64::<0>(v));
            self.s64(svget4_s64::<1>(v));
            self.s64(svget4_s64::<2>(v));
            self.s64(svget4_s64::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn u64(&mut self, v: svuint64_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_u64(svptrue_b64(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn u64x2(&mut self, v: svuint64x2_t) {
        unsafe {
            self.u64(svget2_u64::<0>(v));
            self.u64(svget2_u64::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u64x3(&mut self, v: svuint64x3_t) {
        unsafe {
            self.u64(svget3_u64::<0>(v));
            self.u64(svget3_u64::<1>(v));
            self.u64(svget3_u64::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn u64x4(&mut self, v: svuint64x4_t) {
        unsafe {
            self.u64(svget4_u64::<0>(v));
            self.u64(svget4_u64::<1>(v));
            self.u64(svget4_u64::<2>(v));
            self.u64(svget4_u64::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn f32(&mut self, v: svfloat32_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_f32(svptrue_b32(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn f32x2(&mut self, v: svfloat32x2_t) {
        unsafe {
            self.f32(svget2_f32::<0>(v));
            self.f32(svget2_f32::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f32x3(&mut self, v: svfloat32x3_t) {
        unsafe {
            self.f32(svget3_f32::<0>(v));
            self.f32(svget3_f32::<1>(v));
            self.f32(svget3_f32::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f32x4(&mut self, v: svfloat32x4_t) {
        unsafe {
            self.f32(svget4_f32::<0>(v));
            self.f32(svget4_f32::<1>(v));
            self.f32(svget4_f32::<2>(v));
            self.f32(svget4_f32::<3>(v))
        }
    }

    #[target_feature(enable = "sve")]
    unsafe fn f64(&mut self, v: svfloat64_t) {
        let mut image = [0u8; 256];
        // SAFETY: a vector is at most 256 bytes.
        unsafe { svst1_f64(svptrue_b64(), image.as_mut_ptr().cast(), v) };
        self.bytes(&image[..svcntb() as usize]);
    }
    #[target_feature(enable = "sve")]
    unsafe fn f64x2(&mut self, v: svfloat64x2_t) {
        unsafe {
            self.f64(svget2_f64::<0>(v));
            self.f64(svget2_f64::<1>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f64x3(&mut self, v: svfloat64x3_t) {
        unsafe {
            self.f64(svget3_f64::<0>(v));
            self.f64(svget3_f64::<1>(v));
            self.f64(svget3_f64::<2>(v))
        }
    }
    #[target_feature(enable = "sve")]
    unsafe fn f64x4(&mut self, v: svfloat64x4_t) {
        unsafe {
            self.f64(svget4_f64::<0>(v));
            self.f64(svget4_f64::<1>(v));
            self.f64(svget4_f64::<2>(v));
            self.f64(svget4_f64::<3>(v))
        }
    }
}
#[target_feature(enable = "sve")]
unsafe fn group0(i: &In, o: &mut Out) {
    unsafe {
        o.s8(svabs_s8_m(i.s8(1), i.p(2), i.s8(2)));
        o.s64(svabs_s64_m(i.s64(1), i.p(2), i.s64(2)));
        o.s32(svabs_s32_m(i.s32(0), i.p(1), i.s32(1)));
        o.s16(svabs_s16_m(i.s16(0), i.p(1), i.s16(1)));
        o.s8(svadd_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svadd_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svadd_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svadd_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.u64(svadrb_u64base_s64offset(i.u64(0), i.s64(1)));
        o.u32(svadrb_u32base_s32offset(i.u32(0), i.s32(1)));
        o.u64(svadrd_u64base_s64index(i.u64(1), i.s64(2)));
        o.u32(svadrd_u32base_s32index(i.u32(1), i.s32(2)));
        o.u64(svadrh_u64base_s64index(i.u64(1), i.s64(2)));
        o.u32(svadrh_u32base_s32index(i.u32(1), i.s32(2)));
        o.u64(svadrw_u64base_s64index(i.u64(0), i.s64(1)));
        o.u32(svadrw_u32base_s32index(i.u32(0), i.s32(1)));
        o.s8(svand_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svand_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svand_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svand_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.p(svand_b_z(i.p(0), i.p(1), i.p(2)));
        o.scalar(svandv_s8(i.p(0), i.s8(0)));
        o.scalar(svandv_s64(i.p(0), i.s64(0)));
        o.scalar(svandv_s32(i.p(1), i.s32(1)));
        o.scalar(svandv_s16(i.p(1), i.s16(1)));
        o.s8(svasr_s8_m(i.p(1), i.s8(1), i.u8(2)));
        o.s64(svasr_s64_m(i.p(1), i.s64(1), i.u64(2)));
        o.s32(svasr_s32_m(i.p(0), i.s32(0), i.u32(1)));
        o.s16(svasr_s16_m(i.p(0), i.s16(0), i.u16(1)));
        o.s8(svasr_wide_s8_m(i.p(1), i.s8(1), i.u64(2)));
        o.s32(svasr_wide_s32_m(i.p(0), i.s32(0), i.u64(1)));
        o.s16(svasr_wide_s16_m(i.p(0), i.s16(0), i.u64(1)));
        o.s8(svasrd_n_s8_m::<1>(i.p(0), i.s8(0)));
        o.s8(svasrd_n_s8_m::<8>(i.p(1), i.s8(1)));
        o.s64(svasrd_n_s64_m::<1>(i.p(0), i.s64(0)));
        o.s64(svasrd_n_s64_m::<64>(i.p(1), i.s64(1)));
        o.s32(svasrd_n_s32_m::<1>(i.p(0), i.s32(0)));
        o.s32(svasrd_n_s32_m::<32>(i.p(1), i.s32(1)));
        o.s16(svasrd_n_s16_m::<1>(i.p(0), i.s16(0)));
        o.s16(svasrd_n_s16_m::<16>(i.p(1), i.s16(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group1(i: &In, o: &mut Out) {
    unsafe {
        o.s8(svbic_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svbic_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svbic_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svbic_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.p(svbic_b_z(i.p(1), i.p(2), i.p(3)));
        o.p(svbrka_b_m(i.p(0), i.p(1), i.p(2)));
        o.p(svbrka_b_z(i.p(1), i.p(2)));
        o.p(svbrkb_b_m(i.p(1), i.p(2), i.p(3)));
        o.p(svbrkb_b_z(i.p(0), i.p(1)));
        o.p(svbrkn_b_z(i.p(0), i.p(1), i.p(2)));
        o.p(svbrkpa_b_z(i.p(1), i.p(2), i.p(3)));
        o.p(svbrkpb_b_z(i.p(0), i.p(1), i.p(2)));
        o.scalar(svclasta_n_s8(i.p(0), 1i8, i.s8(0)));
        o.scalar(svclasta_n_f64(i.p(1), f64::NEG_INFINITY, i.f64(1)));
        o.scalar(svclasta_n_s64(i.p(0), -1i64, i.s64(0)));
        o.scalar(svclasta_n_f32(i.p(0), -0.0f32, i.f32(0)));
        o.scalar(svclasta_n_s32(i.p(1), i32::MAX, i.s32(1)));
        o.scalar(svclasta_n_s16(i.p(1), i16::MAX, i.s16(1)));
        o.s8(svclasta_s8(i.p(1), i.s8(1), i.s8(2)));
        o.f64(svclasta_f64(i.p(0), i.f64(0), i.f64(1)));
        o.s64(svclasta_s64(i.p(1), i.s64(1), i.s64(2)));
        o.f32(svclasta_f32(i.p(1), i.f32(1), i.f32(2)));
        o.s32(svclasta_s32(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svclasta_s16(i.p(0), i.s16(0), i.s16(1)));
        o.scalar(svclastb_n_s8(i.p(1), -128i8, i.s8(1)));
        o.scalar(svclastb_n_f64(i.p(0), -0.0f64, i.f64(0)));
        o.scalar(svclastb_n_s64(i.p(1), i64::MAX, i.s64(1)));
        o.scalar(svclastb_n_f32(i.p(1), f32::INFINITY, i.f32(1)));
        o.scalar(svclastb_n_s32(i.p(0), -1i32, i.s32(0)));
        o.scalar(svclastb_n_s16(i.p(0), -1i16, i.s16(0)));
        o.s8(svclastb_s8(i.p(0), i.s8(0), i.s8(1)));
        o.f64(svclastb_f64(i.p(1), i.f64(1), i.f64(2)));
        o.s64(svclastb_s64(i.p(0), i.s64(0), i.s64(1)));
        o.f32(svclastb_f32(i.p(0), i.f32(0), i.f32(1)));
        o.s32(svclastb_s32(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svclastb_s16(i.p(1), i.s16(1), i.s16(2)));
        o.u8(svcls_s8_m(i.u8(1), i.p(2), i.s8(2)));
        o.u64(svcls_s64_m(i.u64(1), i.p(2), i.s64(2)));
        o.u32(svcls_s32_m(i.u32(0), i.p(1), i.s32(1)));
        o.u16(svcls_s16_m(i.u16(0), i.p(1), i.s16(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group2(i: &In, o: &mut Out) {
    unsafe {
        o.u8(svclz_s8_m(i.u8(0), i.p(1), i.s8(1)));
        o.u64(svclz_s64_m(i.u64(0), i.p(1), i.s64(1)));
        o.u32(svclz_s32_m(i.u32(1), i.p(2), i.s32(2)));
        o.u16(svclz_s16_m(i.u16(1), i.p(2), i.s16(2)));
        o.p(svcmpeq_s8(i.p(1), i.s8(1), i.s8(2)));
        o.p(svcmpeq_s64(i.p(1), i.s64(1), i.s64(2)));
        o.p(svcmpeq_s32(i.p(0), i.s32(0), i.s32(1)));
        o.p(svcmpeq_s16(i.p(0), i.s16(0), i.s16(1)));
        o.p(svcmpeq_wide_s8(i.p(1), i.s8(1), i.s64(2)));
        o.p(svcmpeq_wide_s32(i.p(0), i.s32(0), i.s64(1)));
        o.p(svcmpeq_wide_s16(i.p(0), i.s16(0), i.s64(1)));
        o.p(svcmpge_s8(i.p(1), i.s8(1), i.s8(2)));
        o.p(svcmpge_s64(i.p(1), i.s64(1), i.s64(2)));
        o.p(svcmpge_s32(i.p(0), i.s32(0), i.s32(1)));
        o.p(svcmpge_s16(i.p(0), i.s16(0), i.s16(1)));
        o.p(svcmpge_wide_s8(i.p(1), i.s8(1), i.s64(2)));
        o.p(svcmpge_wide_s32(i.p(0), i.s32(0), i.s64(1)));
        o.p(svcmpge_wide_s16(i.p(0), i.s16(0), i.s64(1)));
        o.p(svcmpgt_s8(i.p(0), i.s8(0), i.s8(1)));
        o.p(svcmpgt_s64(i.p(0), i.s64(0), i.s64(1)));
        o.p(svcmpgt_s32(i.p(1), i.s32(1), i.s32(2)));
        o.p(svcmpgt_s16(i.p(1), i.s16(1), i.s16(2)));
        o.p(svcmpgt_wide_s8(i.p(0), i.s8(0), i.s64(1)));
        o.p(svcmpgt_wide_s32(i.p(1), i.s32(1), i.s64(2)));
        o.p(svcmpgt_wide_s16(i.p(1), i.s16(1), i.s64(2)));
        o.p(svcmpgt_u8(i.p(0), i.u8(0), i.u8(1)));
        o.p(svcmpgt_u64(i.p(0), i.u64(0), i.u64(1)));
        o.p(svcmpgt_u32(i.p(1), i.u32(1), i.u32(2)));
        o.p(svcmpgt_u16(i.p(1), i.u16(1), i.u16(2)));
        o.p(svcmpgt_wide_u8(i.p(0), i.u8(0), i.u64(1)));
        o.p(svcmpgt_wide_u32(i.p(1), i.u32(1), i.u64(2)));
        o.p(svcmpgt_wide_u16(i.p(1), i.u16(1), i.u64(2)));
        o.p(svcmpge_u8(i.p(1), i.u8(1), i.u8(2)));
        o.p(svcmpge_u64(i.p(1), i.u64(1), i.u64(2)));
        o.p(svcmpge_u32(i.p(0), i.u32(0), i.u32(1)));
        o.p(svcmpge_u16(i.p(0), i.u16(0), i.u16(1)));
        o.p(svcmpge_wide_u8(i.p(1), i.u8(1), i.u64(2)));
        o.p(svcmpge_wide_u32(i.p(0), i.u32(0), i.u64(1)));
        o.p(svcmpge_wide_u16(i.p(0), i.u16(0), i.u64(1)));
        o.p(svcmple_wide_s8(i.p(0), i.s8(0), i.s64(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group3(i: &In, o: &mut Out) {
    unsafe {
        o.p(svcmple_wide_s32(i.p(1), i.s32(1), i.s64(2)));
        o.p(svcmple_wide_s16(i.p(1), i.s16(1), i.s64(2)));
        o.p(svcmplt_wide_u8(i.p(1), i.u8(1), i.u64(2)));
        o.p(svcmplt_wide_u32(i.p(0), i.u32(0), i.u64(1)));
        o.p(svcmplt_wide_u16(i.p(0), i.u16(0), i.u64(1)));
        o.p(svcmple_wide_u8(i.p(0), i.u8(0), i.u64(1)));
        o.p(svcmple_wide_u32(i.p(1), i.u32(1), i.u64(2)));
        o.p(svcmple_wide_u16(i.p(1), i.u16(1), i.u64(2)));
        o.p(svcmplt_wide_s8(i.p(1), i.s8(1), i.s64(2)));
        o.p(svcmplt_wide_s32(i.p(0), i.s32(0), i.s64(1)));
        o.p(svcmplt_wide_s16(i.p(0), i.s16(0), i.s64(1)));
        o.p(svcmpne_s8(i.p(0), i.s8(0), i.s8(1)));
        o.p(svcmpne_s64(i.p(0), i.s64(0), i.s64(1)));
        o.p(svcmpne_s32(i.p(1), i.s32(1), i.s32(2)));
        o.p(svcmpne_s16(i.p(1), i.s16(1), i.s16(2)));
        o.p(svcmpne_wide_s8(i.p(0), i.s8(0), i.s64(1)));
        o.p(svcmpne_wide_s32(i.p(1), i.s32(1), i.s64(2)));
        o.p(svcmpne_wide_s16(i.p(1), i.s16(1), i.s64(2)));
        o.s8(svcnot_s8_m(i.s8(1), i.p(2), i.s8(2)));
        o.s64(svcnot_s64_m(i.s64(1), i.p(2), i.s64(2)));
        o.s32(svcnot_s32_m(i.s32(0), i.p(1), i.s32(1)));
        o.s16(svcnot_s16_m(i.s16(0), i.p(1), i.s16(1)));
        o.u8(svcnt_s8_m(i.u8(0), i.p(1), i.s8(1)));
        o.u64(svcnt_f64_m(i.u64(1), i.p(2), i.f64(2)));
        o.u64(svcnt_s64_m(i.u64(0), i.p(1), i.s64(1)));
        o.u32(svcnt_f32_m(i.u32(0), i.p(1), i.f32(1)));
        o.u32(svcnt_s32_m(i.u32(1), i.p(2), i.s32(2)));
        o.u16(svcnt_s16_m(i.u16(1), i.p(2), i.s16(2)));
        o.scalar(svcntb_pat::<{ svpattern::SV_VL7 }>());
        o.scalar(svcntb_pat::<{ svpattern::SV_MUL3 }>());
        o.scalar(svcntd_pat::<{ svpattern::SV_VL7 }>());
        o.scalar(svcntd_pat::<{ svpattern::SV_MUL3 }>());
        o.scalar(svcnth_pat::<{ svpattern::SV_VL7 }>());
        o.scalar(svcnth_pat::<{ svpattern::SV_MUL3 }>());
        o.scalar(svcntp_b8(i.p(1), i.p(2)));
        o.scalar(svcntp_b64(i.p(1), i.p(2)));
        o.scalar(svcntp_b32(i.p(0), i.p(1)));
        o.scalar(svcntp_b16(i.p(0), i.p(1)));
        o.scalar(svcntw_pat::<{ svpattern::SV_VL7 }>());
        o.scalar(svcntw_pat::<{ svpattern::SV_MUL3 }>());
    }
}
#[target_feature(enable = "sve")]
unsafe fn group4(i: &In, o: &mut Out) {
    unsafe {
        o.f64(svcompact_f64(i.p(1), i.f64(1)));
        o.s64(svcompact_s64(i.p(0), i.s64(0)));
        o.f32(svcompact_f32(i.p(0), i.f32(0)));
        o.s32(svcompact_s32(i.p(1), i.s32(1)));
        o.s8(svdup_n_s8_m(i.s8(1), i.p(2), 127i8));
        o.f64(svdup_n_f64_m(i.f64(0), i.p(1), f64::NAN));
        o.s64(svdup_n_s64_m(i.s64(1), i.p(2), 0i64));
        o.f32(svdup_n_f32_m(i.f32(1), i.p(2), -3.25e7f32));
        o.s32(svdup_n_s32_m(i.s32(0), i.p(1), i32::MIN));
        o.s16(svdup_n_s16_m(i.s16(0), i.p(1), i16::MIN));
        o.p(svdup_n_b8(true));
        o.s8(svdup_n_s8(-1i8));
        o.f64(svdup_n_f64(2.5f64));
        o.p(svdup_n_b64(true));
        o.s64(svdup_n_s64(i64::MIN));
        o.f32(svdup_n_f32(f32::NAN));
        o.p(svdup_n_b32(true));
        o.s32(svdup_n_s32(3i32));
        o.p(svdup_n_b16(true));
        o.s16(svdup_n_s16(0i16));
        o.s8(svdupq_lane_s8(i.s8(0), 1u64));
        o.f64(svdupq_lane_f64(i.f64(1), 1u64 << 63));
        o.s64(svdupq_lane_s64(i.s64(0), 1u64));
        o.f32(svdupq_lane_f32(i.f32(0), 1u64));
        o.s32(svdupq_lane_s32(i.s32(1), 1u64 << 63));
        o.s16(svdupq_lane_s16(i.s16(1), 1u64 << 63));
        o.s8(sveor_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(sveor_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(sveor_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(sveor_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.p(sveor_b_z(i.p(1), i.p(2), i.p(3)));
        o.scalar(sveorv_s8(i.p(1), i.s8(1)));
        o.scalar(sveorv_s64(i.p(1), i.s64(1)));
        o.scalar(sveorv_s32(i.p(0), i.s32(0)));
        o.scalar(sveorv_s16(i.p(0), i.s16(0)));
        o.s8(svext_s8::<0>(i.s8(0), i.s8(1)));
        o.s8(svext_s8::<255>(i.s8(1), i.s8(2)));
        o.f64(svext_f64::<0>(i.f64(0), i.f64(1)));
        o.f64(svext_f64::<31>(i.f64(1), i.f64(2)));
        o.s64(svext_s64::<0>(i.s64(0), i.s64(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group5(i: &In, o: &mut Out) {
    unsafe {
        o.s64(svext_s64::<31>(i.s64(1), i.s64(2)));
        o.f32(svext_f32::<0>(i.f32(0), i.f32(1)));
        o.f32(svext_f32::<63>(i.f32(1), i.f32(2)));
        o.s32(svext_s32::<0>(i.s32(0), i.s32(1)));
        o.s32(svext_s32::<63>(i.s32(1), i.s32(2)));
        o.s16(svext_s16::<0>(i.s16(0), i.s16(1)));
        o.s16(svext_s16::<127>(i.s16(1), i.s16(2)));
        o.f64(svabd_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svabd_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.f64(svabs_f64_m(i.f64(0), i.p(1), i.f64(1)));
        o.f32(svabs_f32_m(i.f32(1), i.p(2), i.f32(2)));
        o.p(svacge_f64(i.p(0), i.f64(0), i.f64(1)));
        o.p(svacge_f32(i.p(1), i.f32(1), i.f32(2)));
        o.p(svacgt_f64(i.p(1), i.f64(1), i.f64(2)));
        o.p(svacgt_f32(i.p(0), i.f32(0), i.f32(1)));
        o.f64(svadd_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svadd_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.scalar(svadda_f64(i.p(0), -0.0f64, i.f64(0)));
        o.scalar(svadda_f32(i.p(1), f32::INFINITY, i.f32(1)));
        o.scalar(svaddv_f64(i.p(1), i.f64(1)));
        o.scalar(svaddv_f32(i.p(0), i.f32(0)));
        o.f64(svcadd_f64_m::<90>(i.p(0), i.f64(0), i.f64(1)));
        o.f64(svcadd_f64_m::<90>(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svcadd_f32_m::<90>(i.p(0), i.f32(0), i.f32(1)));
        o.f32(svcadd_f32_m::<90>(i.p(1), i.f32(1), i.f32(2)));
        o.f32(svcmla_lane_f32::<0, 0>(i.f32(0), i.f32(1), i.f32(2)));
        o.f32(svcmla_lane_f32::<1, 0>(i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svcmla_f64_m::<0>(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f64(svcmla_f64_m::<0>(i.p(1), i.f64(1), i.f64(2), i.f64(3)));
        o.f32(svcmla_f32_m::<0>(i.p(0), i.f32(0), i.f32(1), i.f32(2)));
        o.f32(svcmla_f32_m::<0>(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.p(svcmpeq_f64(i.p(0), i.f64(0), i.f64(1)));
        o.p(svcmpeq_f32(i.p(1), i.f32(1), i.f32(2)));
        o.p(svcmpge_f64(i.p(0), i.f64(0), i.f64(1)));
        o.p(svcmpge_f32(i.p(1), i.f32(1), i.f32(2)));
        o.p(svcmpgt_f64(i.p(1), i.f64(1), i.f64(2)));
        o.p(svcmpgt_f32(i.p(0), i.f32(0), i.f32(1)));
        o.p(svcmpne_f64(i.p(1), i.f64(1), i.f64(2)));
        o.p(svcmpne_f32(i.p(0), i.f32(0), i.f32(1)));
        o.p(svcmpuo_f64(i.p(0), i.f64(0), i.f64(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group6(i: &In, o: &mut Out) {
    unsafe {
        o.p(svcmpuo_f32(i.p(1), i.f32(1), i.f32(2)));
        o.f32(svcvt_f32_f64_m(i.f32(1), i.p(2), i.f64(2)));
        o.f64(svcvt_f64_f32_m(i.f64(1), i.p(2), i.f32(2)));
        o.s32(svcvt_s32_f64_m(i.s32(0), i.p(1), i.f64(1)));
        o.s64(svcvt_s64_f32_m(i.s64(0), i.p(1), i.f32(1)));
        o.s64(svcvt_s64_f64_m(i.s64(1), i.p(2), i.f64(2)));
        o.s32(svcvt_s32_f32_m(i.s32(1), i.p(2), i.f32(2)));
        o.u32(svcvt_u32_f64_m(i.u32(0), i.p(1), i.f64(1)));
        o.u64(svcvt_u64_f32_m(i.u64(0), i.p(1), i.f32(1)));
        o.u64(svcvt_u64_f64_m(i.u64(1), i.p(2), i.f64(2)));
        o.u32(svcvt_u32_f32_m(i.u32(1), i.p(2), i.f32(2)));
        o.f64(svdiv_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svdiv_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.f64(svdivr_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svdivr_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.f64(svexpa_f64(i.u64(0)));
        o.f32(svexpa_f32(i.u32(1)));
        o.f64(svmad_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f32(svmad_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svmax_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svmax_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svmaxnm_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svmaxnm_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.scalar(svmaxnmv_f64(i.p(1), i.f64(1)));
        o.scalar(svmaxnmv_f32(i.p(0), i.f32(0)));
        o.scalar(svmaxv_f64(i.p(0), i.f64(0)));
        o.scalar(svmaxv_f32(i.p(1), i.f32(1)));
        o.f64(svmin_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svmin_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svminnm_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svminnm_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.scalar(svminnmv_f64(i.p(1), i.f64(1)));
        o.scalar(svminnmv_f32(i.p(0), i.f32(0)));
        o.scalar(svminv_f64(i.p(0), i.f64(0)));
        o.scalar(svminv_f32(i.p(1), i.f32(1)));
        o.f64(svmla_lane_f64::<0>(i.f64(0), i.f64(1), i.f64(2)));
        o.f64(svmla_lane_f64::<1>(i.f64(1), i.f64(2), i.f64(3)));
        o.f32(svmla_lane_f32::<0>(i.f32(0), i.f32(1), i.f32(2)));
        o.f32(svmla_lane_f32::<3>(i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svmla_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group7(i: &In, o: &mut Out) {
    unsafe {
        o.f32(svmla_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svmls_lane_f64::<0>(i.f64(0), i.f64(1), i.f64(2)));
        o.f64(svmls_lane_f64::<1>(i.f64(1), i.f64(2), i.f64(3)));
        o.f32(svmls_lane_f32::<0>(i.f32(0), i.f32(1), i.f32(2)));
        o.f32(svmls_lane_f32::<3>(i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svmls_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f32(svmls_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svmsb_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f32(svmsb_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svmul_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svmul_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svmulx_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svmulx_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svneg_f64_m(i.f64(0), i.p(1), i.f64(1)));
        o.f32(svneg_f32_m(i.f32(1), i.p(2), i.f32(2)));
        o.f64(svnmad_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f32(svnmad_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svnmla_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f32(svnmla_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svnmls_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f32(svnmls_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svnmsb_f64_m(i.p(0), i.f64(0), i.f64(1), i.f64(2)));
        o.f32(svnmsb_f32_m(i.p(1), i.f32(1), i.f32(2), i.f32(3)));
        o.f64(svrecpe_f64(i.f64(1)));
        o.f32(svrecpe_f32(i.f32(0)));
        o.f64(svrecps_f64(i.f64(1), i.f64(2)));
        o.f32(svrecps_f32(i.f32(0), i.f32(1)));
        o.f64(svrecpx_f64_m(i.f64(0), i.p(1), i.f64(1)));
        o.f32(svrecpx_f32_m(i.f32(1), i.p(2), i.f32(2)));
        o.f64(svrinta_f64_m(i.f64(0), i.p(1), i.f64(1)));
        o.f32(svrinta_f32_m(i.f32(1), i.p(2), i.f32(2)));
        o.f64(svrinti_f64_m(i.f64(0), i.p(1), i.f64(1)));
        o.f32(svrinti_f32_m(i.f32(1), i.p(2), i.f32(2)));
        o.f64(svrintm_f64_m(i.f64(0), i.p(1), i.f64(1)));
        o.f32(svrintm_f32_m(i.f32(1), i.p(2), i.f32(2)));
        o.f64(svrintn_f64_m(i.f64(1), i.p(2), i.f64(2)));
        o.f32(svrintn_f32_m(i.f32(0), i.p(1), i.f32(1)));
        o.f64(svrintp_f64_m(i.f64(1), i.p(2), i.f64(2)));
        o.f32(svrintp_f32_m(i.f32(0), i.p(1), i.f32(1)));
        o.f64(svrintx_f64_m(i.f64(1), i.p(2), i.f64(2)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group8(i: &In, o: &mut Out) {
    unsafe {
        o.f32(svrintx_f32_m(i.f32(0), i.p(1), i.f32(1)));
        o.f64(svrintz_f64_m(i.f64(1), i.p(2), i.f64(2)));
        o.f32(svrintz_f32_m(i.f32(0), i.p(1), i.f32(1)));
        o.f64(svrsqrte_f64(i.f64(1)));
        o.f32(svrsqrte_f32(i.f32(0)));
        o.f64(svrsqrts_f64(i.f64(1), i.f64(2)));
        o.f32(svrsqrts_f32(i.f32(0), i.f32(1)));
        o.f64(svscale_f64_m(i.p(0), i.f64(0), i.s64(1)));
        o.f32(svscale_f32_m(i.p(1), i.f32(1), i.s32(2)));
        o.f64(svsqrt_f64_m(i.f64(0), i.p(1), i.f64(1)));
        o.f32(svsqrt_f32_m(i.f32(1), i.p(2), i.f32(2)));
        o.f64(svsub_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svsub_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svsubr_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svsubr_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svtmad_f64::<0>(i.f64(0), i.f64(1)));
        o.f64(svtmad_f64::<7>(i.f64(1), i.f64(2)));
        o.f32(svtmad_f32::<0>(i.f32(0), i.f32(1)));
        o.f32(svtmad_f32::<7>(i.f32(1), i.f32(2)));
        o.f64(svtsmul_f64(i.f64(1), i.u64(2)));
        o.f32(svtsmul_f32(i.f32(0), i.u32(1)));
        o.f64(svtssel_f64(i.f64(1), i.u64(2)));
        o.f32(svtssel_f32(i.f32(0), i.u32(1)));
        o.s8(svindex_s8(-1i8, -128i8));
        o.s64(svindex_s64(i64::MIN, i64::MAX));
        o.s32(svindex_s32(3i32, -1i32));
        o.s16(svindex_s16(0i16, -1i16));
        o.s8(svinsr_n_s8(i.s8(0), 1i8));
        o.f64(svinsr_n_f64(i.f64(1), f64::NEG_INFINITY));
        o.s64(svinsr_n_s64(i.s64(0), -1i64));
        o.f32(svinsr_n_f32(i.f32(0), -0.0f32));
        o.s32(svinsr_n_s32(i.s32(1), i32::MAX));
        o.s16(svinsr_n_s16(i.s16(1), i16::MAX));
        o.scalar(svlasta_s8(i.p(0), i.s8(0)));
        o.scalar(svlasta_f64(i.p(1), i.f64(1)));
        o.scalar(svlasta_s64(i.p(0), i.s64(0)));
        o.scalar(svlasta_f32(i.p(0), i.f32(0)));
        o.scalar(svlasta_s32(i.p(1), i.s32(1)));
        o.scalar(svlasta_s16(i.p(1), i.s16(1)));
        o.scalar(svlastb_s8(i.p(1), i.s8(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group9(i: &In, o: &mut Out) {
    unsafe {
        o.scalar(svlastb_f64(i.p(0), i.f64(0)));
        o.scalar(svlastb_s64(i.p(1), i.s64(1)));
        o.scalar(svlastb_f32(i.p(1), i.f32(1)));
        o.scalar(svlastb_s32(i.p(0), i.s32(0)));
        o.scalar(svlastb_s16(i.p(0), i.s16(0)));
        o.f64(svld1_gather_s64index_f64(i.p(1), i.at::<f64>(64), i.os64(2)));
        o.s64(svld1sh_gather_s64index_s64(i.p(1), i.at::<i16>(64), i.os64(2)));
        o.s64(svld1sw_gather_s64index_s64(i.p(0), i.at::<i32>(64), i.os64(1)));
        o.s64(svld1_gather_s64index_s64(i.p(0), i.at::<i64>(64), i.os64(1)));
        o.f64(svld1_gather_s64offset_f64(i.p(0), i.at::<f64>(64), i.os64(1)));
        o.s64(svld1sh_gather_s64offset_s64(i.p(0), i.at::<i16>(64), i.os64(1)));
        o.s64(svld1sw_gather_s64offset_s64(i.p(1), i.at::<i32>(64), i.os64(2)));
        o.s64(svld1_gather_s64offset_s64(i.p(1), i.at::<i64>(64), i.os64(2)));
        o.s64(svld1sb_gather_s64offset_s64(i.p(0), i.at::<i8>(64), i.os64(1)));
        o.f64(svld1_gather_u64base_offset_f64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.s64(svld1sh_gather_u64base_offset_s64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.s64(svld1sw_gather_u64base_offset_s64(i.p(1), i.bu64(1, i.at::<u8>(1024)), -8));
        o.s64(svld1_gather_u64base_offset_s64(i.p(1), i.bu64(1, i.at::<u8>(1024)), -8));
        o.s64(svld1sb_gather_u64base_offset_s64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.f32(svld1_gather_u32base_offset_f32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.s32(svld1sh_gather_u32base_offset_s32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.s32(svld1_gather_u32base_offset_s32(i.p(1), i.bu32(1), i.at::<u8>(1024) as i64));
        o.s32(svld1sb_gather_u32base_offset_s32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.f32(svld1_gather_s32index_f32(i.p(1), i.at::<f32>(64), i.os32(2)));
        o.s32(svld1sh_gather_s32index_s32(i.p(1), i.at::<i16>(64), i.os32(2)));
        o.s32(svld1_gather_s32index_s32(i.p(0), i.at::<i32>(64), i.os32(1)));
        o.f32(svld1_gather_s32offset_f32(i.p(0), i.at::<f32>(64), i.os32(1)));
        o.s32(svld1sh_gather_s32offset_s32(i.p(0), i.at::<i16>(64), i.os32(1)));
        o.s32(svld1_gather_s32offset_s32(i.p(1), i.at::<i32>(64), i.os32(2)));
        o.s32(svld1sb_gather_s32offset_s32(i.p(0), i.at::<i8>(64), i.os32(1)));
        o.f32(svld1_gather_u32index_f32(i.p(1), i.at::<f32>(64), i.ou32(2)));
        o.s32(svld1sh_gather_u32index_s32(i.p(1), i.at::<i16>(64), i.ou32(2)));
        o.s32(svld1_gather_u32index_s32(i.p(0), i.at::<i32>(64), i.ou32(1)));
        o.f32(svld1_gather_u32offset_f32(i.p(0), i.at::<f32>(64), i.ou32(1)));
        o.s32(svld1sh_gather_u32offset_s32(i.p(0), i.at::<i16>(64), i.ou32(1)));
        o.s32(svld1_gather_u32offset_s32(i.p(1), i.at::<i32>(64), i.ou32(2)));
        o.s32(svld1sb_gather_u32offset_s32(i.p(0), i.at::<i8>(64), i.ou32(1)));
        o.s8(svld1_s8(i.p(0), i.at::<i8>(16)));
        o.f64(svld1_f64(i.p(1), i.at::<f64>(40)));
        o.s64(svld1sh_s64(i.p(1), i.at::<i16>(40)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group10(i: &In, o: &mut Out) {
    unsafe {
        o.s64(svld1sw_s64(i.p(0), i.at::<i32>(16)));
        o.s64(svld1_s64(i.p(0), i.at::<i64>(16)));
        o.s64(svld1sb_s64(i.p(1), i.at::<i8>(40)));
        o.f32(svld1_f32(i.p(0), i.at::<f32>(16)));
        o.s32(svld1sh_s32(i.p(0), i.at::<i16>(16)));
        o.s32(svld1_s32(i.p(1), i.at::<i32>(40)));
        o.s32(svld1sb_s32(i.p(0), i.at::<i8>(16)));
        o.s16(svld1_s16(i.p(1), i.at::<i16>(40)));
        o.s16(svld1sb_s16(i.p(0), i.at::<i8>(16)));
        o.s8(svld1rq_s8(i.p(1), i.at::<i8>(40)));
        o.f64(svld1rq_f64(i.p(0), i.at::<f64>(16)));
        o.s64(svld1rq_s64(i.p(1), i.at::<i64>(40)));
        o.f32(svld1rq_f32(i.p(1), i.at::<f32>(40)));
        o.s32(svld1rq_s32(i.p(0), i.at::<i32>(16)));
        o.s16(svld1rq_s16(i.p(0), i.at::<i16>(16)));
        o.s8x2(svld2_s8(i.p(1), i.at::<i8>(40)));
        o.f64x2(svld2_f64(i.p(0), i.at::<f64>(16)));
        o.s64x2(svld2_s64(i.p(1), i.at::<i64>(40)));
        o.f32x2(svld2_f32(i.p(1), i.at::<f32>(40)));
        o.s32x2(svld2_s32(i.p(0), i.at::<i32>(16)));
        o.s16x2(svld2_s16(i.p(0), i.at::<i16>(16)));
        o.s8x3(svld3_s8(i.p(0), i.at::<i8>(16)));
        o.f64x3(svld3_f64(i.p(1), i.at::<f64>(40)));
        o.s64x3(svld3_s64(i.p(0), i.at::<i64>(16)));
        o.f32x3(svld3_f32(i.p(0), i.at::<f32>(16)));
        o.s32x3(svld3_s32(i.p(1), i.at::<i32>(40)));
        o.s16x3(svld3_s16(i.p(1), i.at::<i16>(40)));
        o.s8x4(svld4_s8(i.p(1), i.at::<i8>(40)));
        o.f64x4(svld4_f64(i.p(0), i.at::<f64>(16)));
        o.s64x4(svld4_s64(i.p(1), i.at::<i64>(40)));
        o.f32x4(svld4_f32(i.p(1), i.at::<f32>(40)));
        o.s32x4(svld4_s32(i.p(0), i.at::<i32>(16)));
        o.s16x4(svld4_s16(i.p(0), i.at::<i16>(16)));
        o.f64(svldff1_gather_s64index_f64(i.p(1), i.at::<f64>(64), i.os64(2)));
        o.s64(svldff1sh_gather_s64index_s64(i.p(1), i.at::<i16>(64), i.os64(2)));
        o.s64(svldff1sw_gather_s64index_s64(i.p(0), i.at::<i32>(64), i.os64(1)));
        o.s64(svldff1_gather_s64index_s64(i.p(0), i.at::<i64>(64), i.os64(1)));
        o.f64(svldff1_gather_s64offset_f64(i.p(0), i.at::<f64>(64), i.os64(1)));
        o.s64(svldff1sh_gather_s64offset_s64(i.p(0), i.at::<i16>(64), i.os64(1)));
        o.s64(svldff1sw_gather_s64offset_s64(i.p(1), i.at::<i32>(64), i.os64(2)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group11(i: &In, o: &mut Out) {
    unsafe {
        o.s64(svldff1_gather_s64offset_s64(i.p(1), i.at::<i64>(64), i.os64(2)));
        o.s64(svldff1sb_gather_s64offset_s64(i.p(0), i.at::<i8>(64), i.os64(1)));
        o.f64(svldff1_gather_u64base_offset_f64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.s64(svldff1sh_gather_u64base_offset_s64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.s64(svldff1sw_gather_u64base_offset_s64(i.p(1), i.bu64(1, i.at::<u8>(1024)), -8));
        o.s64(svldff1_gather_u64base_offset_s64(i.p(1), i.bu64(1, i.at::<u8>(1024)), -8));
        o.s64(svldff1sb_gather_u64base_offset_s64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.f32(svldff1_gather_u32base_offset_f32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.s32(svldff1sh_gather_u32base_offset_s32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.s32(svldff1_gather_u32base_offset_s32(i.p(1), i.bu32(1), i.at::<u8>(1024) as i64));
        o.s32(svldff1sb_gather_u32base_offset_s32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.f32(svldff1_gather_s32index_f32(i.p(1), i.at::<f32>(64), i.os32(2)));
        o.s32(svldff1sh_gather_s32index_s32(i.p(1), i.at::<i16>(64), i.os32(2)));
        o.s32(svldff1_gather_s32index_s32(i.p(0), i.at::<i32>(64), i.os32(1)));
        o.f32(svldff1_gather_s32offset_f32(i.p(0), i.at::<f32>(64), i.os32(1)));
        o.s32(svldff1sh_gather_s32offset_s32(i.p(0), i.at::<i16>(64), i.os32(1)));
        o.s32(svldff1_gather_s32offset_s32(i.p(1), i.at::<i32>(64), i.os32(2)));
        o.s32(svldff1sb_gather_s32offset_s32(i.p(0), i.at::<i8>(64), i.os32(1)));
        o.f32(svldff1_gather_u32index_f32(i.p(1), i.at::<f32>(64), i.ou32(2)));
        o.s32(svldff1sh_gather_u32index_s32(i.p(1), i.at::<i16>(64), i.ou32(2)));
        o.s32(svldff1_gather_u32index_s32(i.p(0), i.at::<i32>(64), i.ou32(1)));
        o.f32(svldff1_gather_u32offset_f32(i.p(0), i.at::<f32>(64), i.ou32(1)));
        o.s32(svldff1sh_gather_u32offset_s32(i.p(0), i.at::<i16>(64), i.ou32(1)));
        o.s32(svldff1_gather_u32offset_s32(i.p(1), i.at::<i32>(64), i.ou32(2)));
        o.s32(svldff1sb_gather_u32offset_s32(i.p(0), i.at::<i8>(64), i.ou32(1)));
        o.s8(svldff1_s8(i.p(0), i.at::<i8>(16)));
        o.f64(svldff1_f64(i.p(1), i.at::<f64>(40)));
        o.s64(svldff1sh_s64(i.p(1), i.at::<i16>(40)));
        o.s64(svldff1sw_s64(i.p(0), i.at::<i32>(16)));
        o.s64(svldff1_s64(i.p(0), i.at::<i64>(16)));
        o.s64(svldff1sb_s64(i.p(1), i.at::<i8>(40)));
        o.f32(svldff1_f32(i.p(0), i.at::<f32>(16)));
        o.s32(svldff1sh_s32(i.p(0), i.at::<i16>(16)));
        o.s32(svldff1_s32(i.p(1), i.at::<i32>(40)));
        o.s32(svldff1sb_s32(i.p(0), i.at::<i8>(16)));
        o.s16(svldff1_s16(i.p(1), i.at::<i16>(40)));
        o.s16(svldff1sb_s16(i.p(0), i.at::<i8>(16)));
        o.s8(svldnf1_s8(i.p(0), i.at::<i8>(16)));
        o.f64(svldnf1_f64(i.p(1), i.at::<f64>(40)));
        o.s64(svldnf1sh_s64(i.p(1), i.at::<i16>(40)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group12(i: &In, o: &mut Out) {
    unsafe {
        o.s64(svldnf1sw_s64(i.p(0), i.at::<i32>(16)));
        o.s64(svldnf1_s64(i.p(0), i.at::<i64>(16)));
        o.s64(svldnf1sb_s64(i.p(1), i.at::<i8>(40)));
        o.f32(svldnf1_f32(i.p(0), i.at::<f32>(16)));
        o.s32(svldnf1sh_s32(i.p(0), i.at::<i16>(16)));
        o.s32(svldnf1_s32(i.p(1), i.at::<i32>(40)));
        o.s32(svldnf1sb_s32(i.p(0), i.at::<i8>(16)));
        o.s16(svldnf1_s16(i.p(1), i.at::<i16>(40)));
        o.s16(svldnf1sb_s16(i.p(0), i.at::<i8>(16)));
        o.s8(svldnt1_s8(i.p(0), i.at::<i8>(16)));
        o.f64(svldnt1_f64(i.p(1), i.at::<f64>(40)));
        o.s64(svldnt1_s64(i.p(0), i.at::<i64>(16)));
        o.f32(svldnt1_f32(i.p(0), i.at::<f32>(16)));
        o.s32(svldnt1_s32(i.p(1), i.at::<i32>(40)));
        o.s16(svldnt1_s16(i.p(1), i.at::<i16>(40)));
        o.s8(svlsl_s8_m(i.p(0), i.s8(0), i.u8(1)));
        o.s64(svlsl_s64_m(i.p(0), i.s64(0), i.u64(1)));
        o.s32(svlsl_s32_m(i.p(1), i.s32(1), i.u32(2)));
        o.s16(svlsl_s16_m(i.p(1), i.s16(1), i.u16(2)));
        o.s8(svlsl_wide_s8_m(i.p(0), i.s8(0), i.u64(1)));
        o.s32(svlsl_wide_s32_m(i.p(1), i.s32(1), i.u64(2)));
        o.s16(svlsl_wide_s16_m(i.p(1), i.s16(1), i.u64(2)));
        o.u8(svlsr_u8_m(i.p(0), i.u8(0), i.u8(1)));
        o.u64(svlsr_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svlsr_u32_m(i.p(1), i.u32(1), i.u32(2)));
        o.u16(svlsr_u16_m(i.p(1), i.u16(1), i.u16(2)));
        o.u8(svlsr_wide_u8_m(i.p(0), i.u8(0), i.u64(1)));
        o.u32(svlsr_wide_u32_m(i.p(1), i.u32(1), i.u64(2)));
        o.u16(svlsr_wide_u16_m(i.p(1), i.u16(1), i.u64(2)));
        o.s8(svmad_s8_m(i.p(1), i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svmad_s64_m(i.p(1), i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svmad_s32_m(i.p(0), i.s32(0), i.s32(1), i.s32(2)));
        o.s16(svmad_s16_m(i.p(0), i.s16(0), i.s16(1), i.s16(2)));
        o.s8(svmla_s8_m(i.p(1), i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svmla_s64_m(i.p(1), i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svmla_s32_m(i.p(0), i.s32(0), i.s32(1), i.s32(2)));
        o.s16(svmla_s16_m(i.p(0), i.s16(0), i.s16(1), i.s16(2)));
        o.s8(svmls_s8_m(i.p(1), i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svmls_s64_m(i.p(1), i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svmls_s32_m(i.p(0), i.s32(0), i.s32(1), i.s32(2)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group13(i: &In, o: &mut Out) {
    unsafe {
        o.s16(svmls_s16_m(i.p(0), i.s16(0), i.s16(1), i.s16(2)));
        o.s8(svmsb_s8_m(i.p(1), i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svmsb_s64_m(i.p(1), i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svmsb_s32_m(i.p(0), i.s32(0), i.s32(1), i.s32(2)));
        o.s16(svmsb_s16_m(i.p(0), i.s16(0), i.s16(1), i.s16(2)));
        o.s8(svmul_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svmul_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svmul_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svmul_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.p(svnand_b_z(i.p(0), i.p(1), i.p(2)));
        o.s8(svneg_s8_m(i.s8(1), i.p(2), i.s8(2)));
        o.s64(svneg_s64_m(i.s64(1), i.p(2), i.s64(2)));
        o.s32(svneg_s32_m(i.s32(0), i.p(1), i.s32(1)));
        o.s16(svneg_s16_m(i.s16(0), i.p(1), i.s16(1)));
        o.p(svnor_b_z(i.p(0), i.p(1), i.p(2)));
        o.s8(svnot_s8_m(i.s8(0), i.p(1), i.s8(1)));
        o.s64(svnot_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svnot_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s16(svnot_s16_m(i.s16(1), i.p(2), i.s16(2)));
        o.p(svorn_b_z(i.p(0), i.p(1), i.p(2)));
        o.s8(svorr_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svorr_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svorr_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svorr_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.p(svorr_b_z(i.p(0), i.p(1), i.p(2)));
        o.scalar(svorv_s8(i.p(0), i.s8(0)));
        o.scalar(svorv_s64(i.p(0), i.s64(0)));
        o.scalar(svorv_s32(i.p(1), i.s32(1)));
        o.scalar(svorv_s16(i.p(1), i.s16(1)));
        o.p(svpfirst_b(i.p(0), i.p(1)));
        o.p(svpnext_b8(i.p(1), i.p(2)));
        o.p(svpnext_b64(i.p(1), i.p(2)));
        o.p(svpnext_b32(i.p(0), i.p(1)));
        o.p(svpnext_b16(i.p(0), i.p(1)));
        svprfb_gather_s64offset::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os64(1));
        o.written();
        svprfb_gather_s64offset::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os64(2));
        o.written();
        svprfb_gather_u64base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu64(0, i.at::<u8>(1024)));
        o.written();
        svprfb_gather_u64base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu64(1, i.at::<u8>(1024)));
        o.written();
        svprfb_gather_u64base_offset::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu64(0, i.at::<u8>(1024)),
            8,
        );
        o.written();
        svprfb_gather_u64base_offset::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu64(1, i.at::<u8>(1024)),
            -8,
        );
        o.written();
    }
}
#[target_feature(enable = "sve")]
unsafe fn group14(i: &In, o: &mut Out) {
    unsafe {
        svprfb_gather_u32base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu32(0));
        o.written();
        svprfb_gather_u32base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu32(1));
        o.written();
        svprfb_gather_u32base_offset::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu32(0),
            i.at::<u8>(1024) as i64,
        );
        o.written();
        svprfb_gather_u32base_offset::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu32(1),
            i.at::<u8>(1024) as i64,
        );
        o.written();
        svprfb_gather_s32offset::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os32(1));
        o.written();
        svprfb_gather_s32offset::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os32(2));
        o.written();
        svprfb_gather_u32offset::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.ou32(1));
        o.written();
        svprfb_gather_u32offset::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.ou32(2));
        o.written();
        svprfd_gather_s64index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os64(1));
        o.written();
        svprfd_gather_s64index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os64(2));
        o.written();
        svprfd_gather_u64base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu64(0, i.at::<u8>(1024)));
        o.written();
        svprfd_gather_u64base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu64(1, i.at::<u8>(1024)));
        o.written();
        svprfd_gather_u64base_index::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu64(0, i.at::<u8>(1024)),
            1,
        );
        o.written();
        svprfd_gather_u64base_index::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu64(1, i.at::<u8>(1024)),
            -1,
        );
        o.written();
        svprfd_gather_u32base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu32(0));
        o.written();
        svprfd_gather_u32base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu32(1));
        o.written();
        svprfd_gather_u32base_index::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu32(0),
            i.at::<u8>(1024) as i64 >> 3,
        );
        o.written();
        svprfd_gather_u32base_index::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu32(1),
            i.at::<u8>(1024) as i64 >> 3,
        );
        o.written();
        svprfd_gather_s32index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os32(1));
        o.written();
        svprfd_gather_s32index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os32(2));
        o.written();
        svprfd_gather_u32index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.ou32(1));
        o.written();
        svprfd_gather_u32index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.ou32(2));
        o.written();
        svprfh_gather_s64index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os64(1));
        o.written();
        svprfh_gather_s64index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os64(2));
        o.written();
        svprfh_gather_u64base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu64(0, i.at::<u8>(1024)));
        o.written();
        svprfh_gather_u64base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu64(1, i.at::<u8>(1024)));
        o.written();
        svprfh_gather_u64base_index::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu64(0, i.at::<u8>(1024)),
            1,
        );
        o.written();
        svprfh_gather_u64base_index::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu64(1, i.at::<u8>(1024)),
            -1,
        );
        o.written();
        svprfh_gather_u32base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu32(0));
        o.written();
        svprfh_gather_u32base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu32(1));
        o.written();
        svprfh_gather_u32base_index::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu32(0),
            i.at::<u8>(1024) as i64 >> 1,
        );
        o.written();
        svprfh_gather_u32base_index::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu32(1),
            i.at::<u8>(1024) as i64 >> 1,
        );
        o.written();
        svprfh_gather_s32index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os32(1));
        o.written();
        svprfh_gather_s32index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os32(2));
        o.written();
        svprfh_gather_u32index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.ou32(1));
        o.written();
        svprfh_gather_u32index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.ou32(2));
        o.written();
        svprfw_gather_s64index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os64(1));
        o.written();
        svprfw_gather_s64index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os64(2));
        o.written();
        svprfw_gather_u64base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu64(0, i.at::<u8>(1024)));
        o.written();
        svprfw_gather_u64base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu64(1, i.at::<u8>(1024)));
        o.written();
    }
}
#[target_feature(enable = "sve")]
unsafe fn group15(i: &In, o: &mut Out) {
    unsafe {
        svprfw_gather_u64base_index::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu64(0, i.at::<u8>(1024)),
            1,
        );
        o.written();
        svprfw_gather_u64base_index::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu64(1, i.at::<u8>(1024)),
            -1,
        );
        o.written();
        svprfw_gather_u32base::<{ svprfop::SV_PLDL1KEEP }>(i.p(0), i.bu32(0));
        o.written();
        svprfw_gather_u32base::<{ svprfop::SV_PSTL3STRM }>(i.p(1), i.bu32(1));
        o.written();
        svprfw_gather_u32base_index::<{ svprfop::SV_PLDL1KEEP }>(
            i.p(0),
            i.bu32(0),
            i.at::<u8>(1024) as i64 >> 2,
        );
        o.written();
        svprfw_gather_u32base_index::<{ svprfop::SV_PSTL3STRM }>(
            i.p(1),
            i.bu32(1),
            i.at::<u8>(1024) as i64 >> 2,
        );
        o.written();
        svprfw_gather_s32index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.os32(1));
        o.written();
        svprfw_gather_s32index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.os32(2));
        o.written();
        svprfw_gather_u32index::<{ svprfop::SV_PLDL1KEEP }, _>(i.p(0), i.at::<u8>(64), i.ou32(1));
        o.written();
        svprfw_gather_u32index::<{ svprfop::SV_PSTL3STRM }, _>(i.p(1), i.at::<u8>(64), i.ou32(2));
        o.written();
        o.scalar(svptest_any(i.p(0), i.p(1)));
        o.scalar(svptest_first(i.p(0), i.p(1)));
        o.scalar(svptest_last(i.p(0), i.p(1)));
        o.p(svptrue_pat_b8::<{ svpattern::SV_VL7 }>());
        o.p(svptrue_pat_b8::<{ svpattern::SV_MUL3 }>());
        o.p(svptrue_pat_b64::<{ svpattern::SV_VL7 }>());
        o.p(svptrue_pat_b64::<{ svpattern::SV_MUL3 }>());
        o.p(svptrue_pat_b32::<{ svpattern::SV_VL7 }>());
        o.p(svptrue_pat_b32::<{ svpattern::SV_MUL3 }>());
        o.p(svptrue_pat_b16::<{ svpattern::SV_VL7 }>());
        o.p(svptrue_pat_b16::<{ svpattern::SV_MUL3 }>());
        o.s8(svrbit_s8_m(i.s8(0), i.p(1), i.s8(1)));
        o.s64(svrbit_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svrbit_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s16(svrbit_s16_m(i.s16(1), i.p(2), i.s16(2)));
        o.p(svrdffr_z(i.p(0)));
        o.p(svrev_b16(i.p(0)));
        o.p(svrev_b32(i.p(0)));
        o.p(svrev_b64(i.p(1)));
        o.s64(svrevb_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svrevb_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s16(svrevb_s16_m(i.s16(1), i.p(2), i.s16(2)));
        o.s64(svrevh_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svrevh_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s64(svrevw_s64_m(i.s64(1), i.p(2), i.s64(2)));
        o.s8(svabd_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svabd_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svabd_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svabd_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.scalar(svaddv_s8(i.p(0), i.s8(0)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group16(i: &In, o: &mut Out) {
    unsafe {
        o.scalar(svaddv_s64(i.p(0), i.s64(0)));
        o.scalar(svaddv_s32(i.p(1), i.s32(1)));
        o.scalar(svaddv_s16(i.p(1), i.s16(1)));
        o.f32(svcvt_f32_s64_m(i.f32(0), i.p(1), i.s64(1)));
        o.f64(svcvt_f64_s32_m(i.f64(0), i.p(1), i.s32(1)));
        o.f64(svcvt_f64_s64_m(i.f64(1), i.p(2), i.s64(2)));
        o.f32(svcvt_f32_s32_m(i.f32(1), i.p(2), i.s32(2)));
        o.s64(svdiv_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svdiv_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s64(svdivr_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svdivr_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s64(svdot_lane_s64::<0>(i.s64(0), i.s16(1), i.s16(2)));
        o.s64(svdot_lane_s64::<1>(i.s64(1), i.s16(2), i.s16(3)));
        o.s32(svdot_lane_s32::<0>(i.s32(0), i.s8(1), i.s8(2)));
        o.s32(svdot_lane_s32::<3>(i.s32(1), i.s8(2), i.s8(3)));
        o.s64(svdot_s64(i.s64(0), i.s16(1), i.s16(2)));
        o.s32(svdot_s32(i.s32(1), i.s8(2), i.s8(3)));
        svsetffr();
        o.written();
        o.s8(svmax_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svmax_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svmax_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svmax_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.scalar(svmaxv_s8(i.p(1), i.s8(1)));
        o.scalar(svmaxv_s64(i.p(1), i.s64(1)));
        o.scalar(svmaxv_s32(i.p(0), i.s32(0)));
        o.scalar(svmaxv_s16(i.p(0), i.s16(0)));
        o.s8(svmin_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svmin_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svmin_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svmin_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.scalar(svminv_s8(i.p(1), i.s8(1)));
        o.scalar(svminv_s64(i.p(1), i.s64(1)));
        o.scalar(svminv_s32(i.p(0), i.s32(0)));
        o.scalar(svminv_s16(i.p(0), i.s16(0)));
        o.s8(svmulh_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svmulh_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svmulh_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svmulh_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s8(svsplice_s8(i.p(1), i.s8(1), i.s8(2)));
        o.f64(svsplice_f64(i.p(0), i.f64(0), i.f64(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group17(i: &In, o: &mut Out) {
    unsafe {
        o.s64(svsplice_s64(i.p(1), i.s64(1), i.s64(2)));
        o.f32(svsplice_f32(i.p(1), i.f32(1), i.f32(2)));
        o.s32(svsplice_s32(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svsplice_s16(i.p(0), i.s16(0), i.s16(1)));
        o.s8(svqadd_s8(i.s8(1), i.s8(2)));
        o.s64(svqadd_s64(i.s64(1), i.s64(2)));
        o.s32(svqadd_s32(i.s32(0), i.s32(1)));
        o.s16(svqadd_s16(i.s16(0), i.s16(1)));
        o.scalar(svqdecb_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqdecb_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqdecb_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqdecb_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.scalar(svqdecd_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqdecd_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqdecd_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqdecd_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.s64(svqdecd_pat_s64::<{ svpattern::SV_VL7 }, 1>(i.s64(0)));
        o.s64(svqdecd_pat_s64::<{ svpattern::SV_MUL3 }, 16>(i.s64(1)));
        o.scalar(svqdech_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqdech_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqdech_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqdech_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.s16(svqdech_pat_s16::<{ svpattern::SV_VL7 }, 1>(i.s16(0)));
        o.s16(svqdech_pat_s16::<{ svpattern::SV_MUL3 }, 16>(i.s16(1)));
        o.scalar(svqdecp_n_s32_b8(i32::MIN, i.p(2)));
        o.scalar(svqdecp_n_s32_b64(i32::MIN, i.p(2)));
        o.scalar(svqdecp_n_s32_b32(3i32, i.p(1)));
        o.scalar(svqdecp_n_s32_b16(3i32, i.p(1)));
        o.scalar(svqdecp_n_s64_b8(2i64, i.p(1)));
        o.scalar(svqdecp_n_s64_b64(2i64, i.p(1)));
        o.scalar(svqdecp_n_s64_b32(i64::MIN, i.p(2)));
        o.scalar(svqdecp_n_s64_b16(i64::MIN, i.p(2)));
        o.s64(svqdecp_s64(i.s64(0), i.p(1)));
        o.s32(svqdecp_s32(i.s32(1), i.p(2)));
        o.s16(svqdecp_s16(i.s16(1), i.p(2)));
        o.scalar(svqdecw_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqdecw_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqdecw_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqdecw_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.s32(svqdecw_pat_s32::<{ svpattern::SV_VL7 }, 1>(i.s32(0)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group18(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svqdecw_pat_s32::<{ svpattern::SV_MUL3 }, 16>(i.s32(1)));
        o.scalar(svqincb_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqincb_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqincb_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqincb_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.scalar(svqincd_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqincd_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqincd_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqincd_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.s64(svqincd_pat_s64::<{ svpattern::SV_VL7 }, 1>(i.s64(0)));
        o.s64(svqincd_pat_s64::<{ svpattern::SV_MUL3 }, 16>(i.s64(1)));
        o.scalar(svqinch_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqinch_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqinch_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqinch_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.s16(svqinch_pat_s16::<{ svpattern::SV_VL7 }, 1>(i.s16(0)));
        o.s16(svqinch_pat_s16::<{ svpattern::SV_MUL3 }, 16>(i.s16(1)));
        o.scalar(svqincp_n_s32_b8(i32::MIN, i.p(2)));
        o.scalar(svqincp_n_s32_b64(i32::MIN, i.p(2)));
        o.scalar(svqincp_n_s32_b32(3i32, i.p(1)));
        o.scalar(svqincp_n_s32_b16(3i32, i.p(1)));
        o.scalar(svqincp_n_s64_b8(2i64, i.p(1)));
        o.scalar(svqincp_n_s64_b64(2i64, i.p(1)));
        o.scalar(svqincp_n_s64_b32(i64::MIN, i.p(2)));
        o.scalar(svqincp_n_s64_b16(i64::MIN, i.p(2)));
        o.s64(svqincp_s64(i.s64(0), i.p(1)));
        o.s32(svqincp_s32(i.s32(1), i.p(2)));
        o.s16(svqincp_s16(i.s16(1), i.p(2)));
        o.scalar(svqincw_pat_n_s32::<{ svpattern::SV_VL7 }, 1>(3i32));
        o.scalar(svqincw_pat_n_s32::<{ svpattern::SV_MUL3 }, 16>(i32::MIN));
        o.scalar(svqincw_pat_n_s64::<{ svpattern::SV_VL7 }, 1>(2i64));
        o.scalar(svqincw_pat_n_s64::<{ svpattern::SV_MUL3 }, 16>(i64::MIN));
        o.s32(svqincw_pat_s32::<{ svpattern::SV_VL7 }, 1>(i.s32(0)));
        o.s32(svqincw_pat_s32::<{ svpattern::SV_MUL3 }, 16>(i.s32(1)));
        o.s8(svqsub_s8(i.s8(0), i.s8(1)));
        o.s64(svqsub_s64(i.s64(0), i.s64(1)));
        o.s32(svqsub_s32(i.s32(1), i.s32(2)));
        o.s16(svqsub_s16(i.s16(1), i.s16(2)));
        svst1_s8(i.p(1), o.at::<i8>(24), i.s8(1));
        o.written();
        svst1_f64(i.p(0), o.at::<f64>(8), i.f64(0));
        o.written();
    }
}
#[target_feature(enable = "sve")]
unsafe fn group19(i: &In, o: &mut Out) {
    unsafe {
        svst1h_s64(i.p(1), o.at::<i16>(24), i.s64(1));
        o.written();
        svst1w_s64(i.p(0), o.at::<i32>(8), i.s64(0));
        o.written();
        svst1_s64(i.p(1), o.at::<i64>(24), i.s64(1));
        o.written();
        svst1b_s64(i.p(1), o.at::<i8>(24), i.s64(1));
        o.written();
        svst1_f32(i.p(1), o.at::<f32>(24), i.f32(1));
        o.written();
        svst1h_s32(i.p(0), o.at::<i16>(8), i.s32(0));
        o.written();
        svst1_s32(i.p(0), o.at::<i32>(8), i.s32(0));
        o.written();
        svst1b_s32(i.p(0), o.at::<i8>(8), i.s32(0));
        o.written();
        svst1_s16(i.p(0), o.at::<i16>(8), i.s16(0));
        o.written();
        svst1b_s16(i.p(0), o.at::<i8>(8), i.s16(0));
        o.written();
        svst1_scatter_s64index_f64(i.p(1), o.at::<f64>(64), i.os64(2), i.f64(1));
        o.written();
        svst1h_scatter_s64index_s64(i.p(0), o.at::<i16>(64), i.os64(1), i.s64(0));
        o.written();
        svst1w_scatter_s64index_s64(i.p(1), o.at::<i32>(64), i.os64(2), i.s64(1));
        o.written();
        svst1_scatter_s64index_s64(i.p(0), o.at::<i64>(64), i.os64(1), i.s64(0));
        o.written();
        svst1_scatter_s64offset_f64(i.p(0), o.at::<f64>(64), i.os64(1), i.f64(0));
        o.written();
        svst1h_scatter_s64offset_s64(i.p(1), o.at::<i16>(64), i.os64(2), i.s64(1));
        o.written();
        svst1w_scatter_s64offset_s64(i.p(0), o.at::<i32>(64), i.os64(1), i.s64(0));
        o.written();
        svst1_scatter_s64offset_s64(i.p(1), o.at::<i64>(64), i.os64(2), i.s64(1));
        o.written();
        svst1b_scatter_s64offset_s64(i.p(1), o.at::<i8>(64), i.os64(2), i.s64(1));
        o.written();
        svst1_scatter_u64base_offset_f64(i.p(0), i.bu64(0, o.at::<u8>(1024)), 8, i.f64(0));
        o.written();
        svst1h_scatter_u64base_offset_s64(i.p(1), i.bu64(1, o.at::<u8>(1024)), -8, i.s64(1));
        o.written();
        svst1w_scatter_u64base_offset_s64(i.p(0), i.bu64(0, o.at::<u8>(1024)), 8, i.s64(0));
        o.written();
        svst1_scatter_u64base_offset_s64(i.p(1), i.bu64(1, o.at::<u8>(1024)), -8, i.s64(1));
        o.written();
        svst1b_scatter_u64base_offset_s64(i.p(1), i.bu64(1, o.at::<u8>(1024)), -8, i.s64(1));
        o.written();
        svst1_scatter_u32base_offset_f32(i.p(0), i.bu32(0), o.at::<u8>(1024) as i64, i.f32(0));
        o.written();
        svst1h_scatter_u32base_offset_s32(i.p(1), i.bu32(1), o.at::<u8>(1024) as i64, i.s32(1));
        o.written();
        svst1_scatter_u32base_offset_s32(i.p(1), i.bu32(1), o.at::<u8>(1024) as i64, i.s32(1));
        o.written();
        svst1b_scatter_u32base_offset_s32(i.p(1), i.bu32(1), o.at::<u8>(1024) as i64, i.s32(1));
        o.written();
        svst1_scatter_s32index_f32(i.p(1), o.at::<f32>(64), i.os32(2), i.f32(1));
        o.written();
        svst1h_scatter_s32index_s32(i.p(0), o.at::<i16>(64), i.os32(1), i.s32(0));
        o.written();
        svst1_scatter_s32index_s32(i.p(0), o.at::<i32>(64), i.os32(1), i.s32(0));
        o.written();
        svst1_scatter_s32offset_f32(i.p(0), o.at::<f32>(64), i.os32(1), i.f32(0));
        o.written();
        svst1h_scatter_s32offset_s32(i.p(1), o.at::<i16>(64), i.os32(2), i.s32(1));
        o.written();
        svst1_scatter_s32offset_s32(i.p(1), o.at::<i32>(64), i.os32(2), i.s32(1));
        o.written();
        svst1b_scatter_s32offset_s32(i.p(1), o.at::<i8>(64), i.os32(2), i.s32(1));
        o.written();
        svst1_scatter_u32index_f32(i.p(1), o.at::<f32>(64), i.ou32(2), i.f32(1));
        o.written();
        svst1h_scatter_u32index_s32(i.p(0), o.at::<i16>(64), i.ou32(1), i.s32(0));
        o.written();
        svst1_scatter_u32index_s32(i.p(0), o.at::<i32>(64), i.ou32(1), i.s32(0));
        o.written();
        svst1_scatter_u32offset_f32(i.p(0), o.at::<f32>(64), i.ou32(1), i.f32(0));
        o.written();
        svst1h_scatter_u32offset_s32(i.p(1), o.at::<i16>(64), i.ou32(2), i.s32(1));
        o.written();
    }
}
#[target_feature(enable = "sve")]
unsafe fn group20(i: &In, o: &mut Out) {
    unsafe {
        svst1_scatter_u32offset_s32(i.p(1), o.at::<i32>(64), i.ou32(2), i.s32(1));
        o.written();
        svst1b_scatter_u32offset_s32(i.p(1), o.at::<i8>(64), i.ou32(2), i.s32(1));
        o.written();
        svst2_s8(i.p(0), o.at::<i8>(8), i.s8x2(0));
        o.written();
        svst2_f64(i.p(1), o.at::<f64>(24), i.f64x2(1));
        o.written();
        svst2_s64(i.p(0), o.at::<i64>(8), i.s64x2(0));
        o.written();
        svst2_f32(i.p(0), o.at::<f32>(8), i.f32x2(0));
        o.written();
        svst2_s32(i.p(1), o.at::<i32>(24), i.s32x2(1));
        o.written();
        svst2_s16(i.p(1), o.at::<i16>(24), i.s16x2(1));
        o.written();
        svst3_s8(i.p(1), o.at::<i8>(24), i.s8x3(1));
        o.written();
        svst3_f64(i.p(0), o.at::<f64>(8), i.f64x3(0));
        o.written();
        svst3_s64(i.p(1), o.at::<i64>(24), i.s64x3(1));
        o.written();
        svst3_f32(i.p(1), o.at::<f32>(24), i.f32x3(1));
        o.written();
        svst3_s32(i.p(0), o.at::<i32>(8), i.s32x3(0));
        o.written();
        svst3_s16(i.p(0), o.at::<i16>(8), i.s16x3(0));
        o.written();
        svst4_s8(i.p(0), o.at::<i8>(8), i.s8x4(0));
        o.written();
        svst4_f64(i.p(1), o.at::<f64>(24), i.f64x4(1));
        o.written();
        svst4_s64(i.p(0), o.at::<i64>(8), i.s64x4(0));
        o.written();
        svst4_f32(i.p(0), o.at::<f32>(8), i.f32x4(0));
        o.written();
        svst4_s32(i.p(1), o.at::<i32>(24), i.s32x4(1));
        o.written();
        svst4_s16(i.p(1), o.at::<i16>(24), i.s16x4(1));
        o.written();
        svstnt1_s8(i.p(1), o.at::<i8>(24), i.s8(1));
        o.written();
        svstnt1_f64(i.p(0), o.at::<f64>(8), i.f64(0));
        o.written();
        svstnt1_s64(i.p(1), o.at::<i64>(24), i.s64(1));
        o.written();
        svstnt1_f32(i.p(1), o.at::<f32>(24), i.f32(1));
        o.written();
        svstnt1_s32(i.p(0), o.at::<i32>(8), i.s32(0));
        o.written();
        svstnt1_s16(i.p(0), o.at::<i16>(8), i.s16(0));
        o.written();
        o.s8(svsub_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svsub_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svsub_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svsub_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s8(svsubr_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svsubr_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svsubr_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svsubr_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s64(svextb_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svextb_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s16(svextb_s16_m(i.s16(1), i.p(2), i.s16(2)));
        o.s64(svexth_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svexth_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s64(svextw_s64_m(i.s64(1), i.p(2), i.s64(2)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group21(i: &In, o: &mut Out) {
    unsafe {
        o.s8(svtbl_s8(i.s8(1), i.u8(2)));
        o.f64(svtbl_f64(i.f64(0), i.u64(1)));
        o.s64(svtbl_s64(i.s64(1), i.u64(2)));
        o.f32(svtbl_f32(i.f32(1), i.u32(2)));
        o.s32(svtbl_s32(i.s32(0), i.u32(1)));
        o.s16(svtbl_s16(i.s16(0), i.u16(1)));
        o.p(svtrn1_b16(i.p(0), i.p(1)));
        o.p(svtrn1_b32(i.p(0), i.p(1)));
        o.p(svtrn1_b64(i.p(1), i.p(2)));
        o.p(svtrn1_b8(i.p(1), i.p(2)));
        o.s8(svtrn1_s8(i.s8(0), i.s8(1)));
        o.f64(svtrn1_f64(i.f64(1), i.f64(2)));
        o.s64(svtrn1_s64(i.s64(0), i.s64(1)));
        o.f32(svtrn1_f32(i.f32(0), i.f32(1)));
        o.s32(svtrn1_s32(i.s32(1), i.s32(2)));
        o.s16(svtrn1_s16(i.s16(1), i.s16(2)));
        o.p(svtrn2_b16(i.p(1), i.p(2)));
        o.p(svtrn2_b32(i.p(1), i.p(2)));
        o.p(svtrn2_b64(i.p(0), i.p(1)));
        o.p(svtrn2_b8(i.p(0), i.p(1)));
        o.s8(svtrn2_s8(i.s8(1), i.s8(2)));
        o.f64(svtrn2_f64(i.f64(0), i.f64(1)));
        o.s64(svtrn2_s64(i.s64(1), i.s64(2)));
        o.f32(svtrn2_f32(i.f32(1), i.f32(2)));
        o.s32(svtrn2_s32(i.s32(0), i.s32(1)));
        o.s16(svtrn2_s16(i.s16(0), i.s16(1)));
        o.u8(svabd_u8_m(i.p(0), i.u8(0), i.u8(1)));
        o.u64(svabd_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svabd_u32_m(i.p(1), i.u32(1), i.u32(2)));
        o.u16(svabd_u16_m(i.p(1), i.u16(1), i.u16(2)));
        o.scalar(svaddv_u8(i.p(0), i.u8(0)));
        o.scalar(svaddv_u64(i.p(0), i.u64(0)));
        o.scalar(svaddv_u32(i.p(1), i.u32(1)));
        o.scalar(svaddv_u16(i.p(1), i.u16(1)));
        o.f32(svcvt_f32_u64_m(i.f32(0), i.p(1), i.u64(1)));
        o.f64(svcvt_f64_u32_m(i.f64(0), i.p(1), i.u32(1)));
        o.f64(svcvt_f64_u64_m(i.f64(1), i.p(2), i.u64(2)));
        o.f32(svcvt_f32_u32_m(i.f32(1), i.p(2), i.u32(2)));
        o.u64(svdiv_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svdiv_u32_m(i.p(1), i.u32(1), i.u32(2)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group22(i: &In, o: &mut Out) {
    unsafe {
        o.u64(svdivr_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svdivr_u32_m(i.p(1), i.u32(1), i.u32(2)));
        o.u64(svdot_lane_u64::<0>(i.u64(0), i.u16(1), i.u16(2)));
        o.u64(svdot_lane_u64::<1>(i.u64(1), i.u16(2), i.u16(3)));
        o.u32(svdot_lane_u32::<0>(i.u32(0), i.u8(1), i.u8(2)));
        o.u32(svdot_lane_u32::<3>(i.u32(1), i.u8(2), i.u8(3)));
        o.u64(svdot_u64(i.u64(0), i.u16(1), i.u16(2)));
        o.u32(svdot_u32(i.u32(1), i.u8(2), i.u8(3)));
        o.u8(svmax_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svmax_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svmax_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svmax_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.scalar(svmaxv_u8(i.p(1), i.u8(1)));
        o.scalar(svmaxv_u64(i.p(1), i.u64(1)));
        o.scalar(svmaxv_u32(i.p(0), i.u32(0)));
        o.scalar(svmaxv_u16(i.p(0), i.u16(0)));
        o.u8(svmin_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svmin_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svmin_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svmin_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.scalar(svminv_u8(i.p(1), i.u8(1)));
        o.scalar(svminv_u64(i.p(1), i.u64(1)));
        o.scalar(svminv_u32(i.p(0), i.u32(0)));
        o.scalar(svminv_u16(i.p(0), i.u16(0)));
        o.u8(svmulh_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svmulh_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svmulh_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svmulh_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.u8(svqadd_u8(i.u8(1), i.u8(2)));
        o.u64(svqadd_u64(i.u64(1), i.u64(2)));
        o.u32(svqadd_u32(i.u32(0), i.u32(1)));
        o.u16(svqadd_u16(i.u16(0), i.u16(1)));
        o.scalar(svqdecb_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqdecb_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqdecb_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqdecb_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
        o.scalar(svqdecd_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqdecd_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqdecd_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqdecd_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group23(i: &In, o: &mut Out) {
    unsafe {
        o.u64(svqdecd_pat_u64::<{ svpattern::SV_VL7 }, 1>(i.u64(0)));
        o.u64(svqdecd_pat_u64::<{ svpattern::SV_MUL3 }, 16>(i.u64(1)));
        o.scalar(svqdech_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqdech_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqdech_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqdech_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
        o.u16(svqdech_pat_u16::<{ svpattern::SV_VL7 }, 1>(i.u16(0)));
        o.u16(svqdech_pat_u16::<{ svpattern::SV_MUL3 }, 16>(i.u16(1)));
        o.scalar(svqdecp_n_u32_b8(u32::MAX, i.p(2)));
        o.scalar(svqdecp_n_u32_b64(u32::MAX, i.p(2)));
        o.scalar(svqdecp_n_u32_b32(5u32, i.p(1)));
        o.scalar(svqdecp_n_u32_b16(5u32, i.p(1)));
        o.scalar(svqdecp_n_u64_b8(9u64, i.p(1)));
        o.scalar(svqdecp_n_u64_b64(9u64, i.p(1)));
        o.scalar(svqdecp_n_u64_b32(u64::MAX, i.p(2)));
        o.scalar(svqdecp_n_u64_b16(u64::MAX, i.p(2)));
        o.u64(svqdecp_u64(i.u64(0), i.p(1)));
        o.u32(svqdecp_u32(i.u32(1), i.p(2)));
        o.u16(svqdecp_u16(i.u16(1), i.p(2)));
        o.scalar(svqdecw_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqdecw_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqdecw_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqdecw_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
        o.u32(svqdecw_pat_u32::<{ svpattern::SV_VL7 }, 1>(i.u32(0)));
        o.u32(svqdecw_pat_u32::<{ svpattern::SV_MUL3 }, 16>(i.u32(1)));
        o.scalar(svqincb_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqincb_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqincb_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqincb_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
        o.scalar(svqincd_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqincd_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqincd_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqincd_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
        o.u64(svqincd_pat_u64::<{ svpattern::SV_VL7 }, 1>(i.u64(0)));
        o.u64(svqincd_pat_u64::<{ svpattern::SV_MUL3 }, 16>(i.u64(1)));
        o.scalar(svqinch_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqinch_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqinch_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqinch_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
        o.u16(svqinch_pat_u16::<{ svpattern::SV_VL7 }, 1>(i.u16(0)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group24(i: &In, o: &mut Out) {
    unsafe {
        o.u16(svqinch_pat_u16::<{ svpattern::SV_MUL3 }, 16>(i.u16(1)));
        o.scalar(svqincp_n_u32_b8(u32::MAX, i.p(2)));
        o.scalar(svqincp_n_u32_b64(u32::MAX, i.p(2)));
        o.scalar(svqincp_n_u32_b32(5u32, i.p(1)));
        o.scalar(svqincp_n_u32_b16(5u32, i.p(1)));
        o.scalar(svqincp_n_u64_b8(9u64, i.p(1)));
        o.scalar(svqincp_n_u64_b64(9u64, i.p(1)));
        o.scalar(svqincp_n_u64_b32(u64::MAX, i.p(2)));
        o.scalar(svqincp_n_u64_b16(u64::MAX, i.p(2)));
        o.u64(svqincp_u64(i.u64(0), i.p(1)));
        o.u32(svqincp_u32(i.u32(1), i.p(2)));
        o.u16(svqincp_u16(i.u16(1), i.p(2)));
        o.scalar(svqincw_pat_n_u32::<{ svpattern::SV_VL7 }, 1>(5u32));
        o.scalar(svqincw_pat_n_u32::<{ svpattern::SV_MUL3 }, 16>(u32::MAX));
        o.scalar(svqincw_pat_n_u64::<{ svpattern::SV_VL7 }, 1>(9u64));
        o.scalar(svqincw_pat_n_u64::<{ svpattern::SV_MUL3 }, 16>(u64::MAX));
        o.u32(svqincw_pat_u32::<{ svpattern::SV_VL7 }, 1>(i.u32(0)));
        o.u32(svqincw_pat_u32::<{ svpattern::SV_MUL3 }, 16>(i.u32(1)));
        o.u8(svqsub_u8(i.u8(0), i.u8(1)));
        o.u64(svqsub_u64(i.u64(0), i.u64(1)));
        o.u32(svqsub_u32(i.u32(1), i.u32(2)));
        o.u16(svqsub_u16(i.u16(1), i.u16(2)));
        o.u64(svextb_u64_m(i.u64(0), i.p(1), i.u64(1)));
        o.u32(svextb_u32_m(i.u32(1), i.p(2), i.u32(2)));
        o.u16(svextb_u16_m(i.u16(1), i.p(2), i.u16(2)));
        o.u64(svexth_u64_m(i.u64(0), i.p(1), i.u64(1)));
        o.u32(svexth_u32_m(i.u32(1), i.p(2), i.u32(2)));
        o.u64(svextw_u64_m(i.u64(1), i.p(2), i.u64(2)));
        o.p(svuzp1_b16(i.p(1), i.p(2)));
        o.p(svuzp1_b32(i.p(1), i.p(2)));
        o.p(svuzp1_b64(i.p(0), i.p(1)));
        o.p(svuzp1_b8(i.p(0), i.p(1)));
        o.s8(svuzp1_s8(i.s8(1), i.s8(2)));
        o.f64(svuzp1_f64(i.f64(0), i.f64(1)));
        o.s64(svuzp1_s64(i.s64(1), i.s64(2)));
        o.f32(svuzp1_f32(i.f32(1), i.f32(2)));
        o.s32(svuzp1_s32(i.s32(0), i.s32(1)));
        o.s16(svuzp1_s16(i.s16(0), i.s16(1)));
        o.p(svuzp2_b16(i.p(0), i.p(1)));
        o.p(svuzp2_b32(i.p(0), i.p(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group25(i: &In, o: &mut Out) {
    unsafe {
        o.p(svuzp2_b64(i.p(1), i.p(2)));
        o.p(svuzp2_b8(i.p(1), i.p(2)));
        o.s8(svuzp2_s8(i.s8(0), i.s8(1)));
        o.f64(svuzp2_f64(i.f64(1), i.f64(2)));
        o.s64(svuzp2_s64(i.s64(0), i.s64(1)));
        o.f32(svuzp2_f32(i.f32(0), i.f32(1)));
        o.s32(svuzp2_s32(i.s32(1), i.s32(2)));
        o.s16(svuzp2_s16(i.s16(1), i.s16(2)));
        o.p(svrev_b8(i.p(1)));
        o.s8(svrev_s8(i.s8(0)));
        o.f64(svrev_f64(i.f64(1)));
        o.s64(svrev_s64(i.s64(0)));
        o.f32(svrev_f32(i.f32(0)));
        o.s32(svrev_s32(i.s32(1)));
        o.s16(svrev_s16(i.s16(1)));
        o.p(svwhilele_b8_s32(i32::MIN, i32::MAX));
        o.p(svwhilele_b8_s64(2i64, -1i64));
        o.p(svwhilele_b64_s32(i32::MIN, i32::MAX));
        o.p(svwhilele_b64_s64(2i64, -1i64));
        o.p(svwhilele_b32_s32(3i32, -1i32));
        o.p(svwhilele_b32_s64(i64::MIN, i64::MAX));
        o.p(svwhilele_b16_s32(3i32, -1i32));
        o.p(svwhilele_b16_s64(i64::MIN, i64::MAX));
        o.p(svwhilelt_b8_u32(5u32, 1u32));
        o.p(svwhilelt_b8_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilelt_b64_u32(5u32, 1u32));
        o.p(svwhilelt_b64_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilelt_b32_u32(u32::MAX, 0x8000_0000u32));
        o.p(svwhilelt_b32_u64(9u64, 1u64));
        o.p(svwhilelt_b16_u32(u32::MAX, 0x8000_0000u32));
        o.p(svwhilelt_b16_u64(9u64, 1u64));
        o.p(svwhilele_b8_u32(u32::MAX, 0x8000_0000u32));
        o.p(svwhilele_b8_u64(9u64, 1u64));
        o.p(svwhilele_b64_u32(u32::MAX, 0x8000_0000u32));
        o.p(svwhilele_b64_u64(9u64, 1u64));
        o.p(svwhilele_b32_u32(5u32, 1u32));
        o.p(svwhilele_b32_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilele_b16_u32(5u32, 1u32));
        o.p(svwhilele_b16_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilelt_b8_s32(3i32, -1i32));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group26(i: &In, o: &mut Out) {
    unsafe {
        o.p(svwhilelt_b8_s64(i64::MIN, i64::MAX));
        o.p(svwhilelt_b64_s32(3i32, -1i32));
        o.p(svwhilelt_b64_s64(i64::MIN, i64::MAX));
        o.p(svwhilelt_b32_s32(i32::MIN, i32::MAX));
        o.p(svwhilelt_b32_s64(2i64, -1i64));
        o.p(svwhilelt_b16_s32(i32::MIN, i32::MAX));
        o.p(svwhilelt_b16_s64(2i64, -1i64));
        svwrffr(i.p(0));
        o.written();
        o.p(svzip1_b16(i.p(1), i.p(2)));
        o.p(svzip1_b32(i.p(1), i.p(2)));
        o.p(svzip1_b64(i.p(0), i.p(1)));
        o.p(svzip1_b8(i.p(0), i.p(1)));
        o.s8(svzip1_s8(i.s8(1), i.s8(2)));
        o.f64(svzip1_f64(i.f64(0), i.f64(1)));
        o.s64(svzip1_s64(i.s64(1), i.s64(2)));
        o.f32(svzip1_f32(i.f32(1), i.f32(2)));
        o.s32(svzip1_s32(i.s32(0), i.s32(1)));
        o.s16(svzip1_s16(i.s16(0), i.s16(1)));
        o.p(svzip2_b16(i.p(0), i.p(1)));
        o.p(svzip2_b32(i.p(0), i.p(1)));
        o.p(svzip2_b64(i.p(1), i.p(2)));
        o.p(svzip2_b8(i.p(1), i.p(2)));
        o.s8(svzip2_s8(i.s8(0), i.s8(1)));
        o.f64(svzip2_f64(i.f64(1), i.f64(2)));
        o.s64(svzip2_s64(i.s64(0), i.s64(1)));
        o.f32(svzip2_f32(i.f32(0), i.f32(1)));
        o.s32(svzip2_s32(i.s32(1), i.s32(2)));
        o.s16(svzip2_s16(i.s16(1), i.s16(2)));
    }
}
#[target_feature(enable = "sve,f32mm")]
unsafe fn group27(i: &In, o: &mut Out) {
    unsafe {
        o.f32(svmmla_f32(i.f32(0), i.f32(1), i.f32(2)));
    }
}
#[target_feature(enable = "sve,f64mm")]
unsafe fn group28(i: &In, o: &mut Out) {
    unsafe {
        o.f64(svmmla_f64(i.f64(1), i.f64(2), i.f64(3)));
        o.s8(svld1ro_s8(i.p(1), i.at::<i8>(40)));
        o.f64(svld1ro_f64(i.p(0), i.at::<f64>(16)));
        o.s64(svld1ro_s64(i.p(1), i.at::<i64>(40)));
        o.f32(svld1ro_f32(i.p(1), i.at::<f32>(40)));
        o.s32(svld1ro_s32(i.p(0), i.at::<i32>(16)));
        o.s16(svld1ro_s16(i.p(0), i.at::<i16>(16)));
        o.s8(svtrn1q_s8(i.s8(1), i.s8(2)));
        o.f64(svtrn1q_f64(i.f64(0), i.f64(1)));
        o.s64(svtrn1q_s64(i.s64(1), i.s64(2)));
        o.f32(svtrn1q_f32(i.f32(1), i.f32(2)));
        o.s32(svtrn1q_s32(i.s32(0), i.s32(1)));
        o.s16(svtrn1q_s16(i.s16(0), i.s16(1)));
        o.s8(svtrn2q_s8(i.s8(0), i.s8(1)));
        o.f64(svtrn2q_f64(i.f64(1), i.f64(2)));
        o.s64(svtrn2q_s64(i.s64(0), i.s64(1)));
        o.f32(svtrn2q_f32(i.f32(0), i.f32(1)));
        o.s32(svtrn2q_s32(i.s32(1), i.s32(2)));
        o.s16(svtrn2q_s16(i.s16(1), i.s16(2)));
        o.s8(svuzp1q_s8(i.s8(0), i.s8(1)));
        o.f64(svuzp1q_f64(i.f64(1), i.f64(2)));
        o.s64(svuzp1q_s64(i.s64(0), i.s64(1)));
        o.f32(svuzp1q_f32(i.f32(0), i.f32(1)));
        o.s32(svuzp1q_s32(i.s32(1), i.s32(2)));
        o.s16(svuzp1q_s16(i.s16(1), i.s16(2)));
        o.s8(svuzp2q_s8(i.s8(1), i.s8(2)));
        o.f64(svuzp2q_f64(i.f64(0), i.f64(1)));
        o.s64(svuzp2q_s64(i.s64(1), i.s64(2)));
        o.f32(svuzp2q_f32(i.f32(1), i.f32(2)));
        o.s32(svuzp2q_s32(i.s32(0), i.s32(1)));
        o.s16(svuzp2q_s16(i.s16(0), i.s16(1)));
        o.s8(svzip1q_s8(i.s8(0), i.s8(1)));
        o.f64(svzip1q_f64(i.f64(1), i.f64(2)));
        o.s64(svzip1q_s64(i.s64(0), i.s64(1)));
        o.f32(svzip1q_f32(i.f32(0), i.f32(1)));
        o.s32(svzip1q_s32(i.s32(1), i.s32(2)));
        o.s16(svzip1q_s16(i.s16(1), i.s16(2)));
        o.s8(svzip2q_s8(i.s8(1), i.s8(2)));
        o.f64(svzip2q_f64(i.f64(0), i.f64(1)));
        o.s64(svzip2q_s64(i.s64(1), i.s64(2)));
    }
}
#[target_feature(enable = "sve,f64mm")]
unsafe fn group29(i: &In, o: &mut Out) {
    unsafe {
        o.f32(svzip2q_f32(i.f32(1), i.f32(2)));
        o.s32(svzip2q_s32(i.s32(0), i.s32(1)));
        o.s16(svzip2q_s16(i.s16(0), i.s16(1)));
    }
}
#[target_feature(enable = "sve,i8mm")]
unsafe fn group30(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svmmla_s32(i.s32(1), i.s8(2), i.s8(3)));
        o.s32(svsudot_lane_s32::<0>(i.s32(0), i.s8(1), i.u8(2)));
        o.s32(svsudot_lane_s32::<3>(i.s32(1), i.s8(2), i.u8(3)));
        o.u32(svmmla_u32(i.u32(1), i.u8(2), i.u8(3)));
        o.s32(svusdot_lane_s32::<0>(i.s32(0), i.u8(1), i.s8(2)));
        o.s32(svusdot_lane_s32::<3>(i.s32(1), i.u8(2), i.s8(3)));
        o.s32(svusdot_s32(i.s32(1), i.u8(2), i.s8(3)));
        o.s32(svusmmla_s32(i.s32(1), i.u8(2), i.s8(3)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group31(i: &In, o: &mut Out) {
    unsafe {
        o.u64(svadclb_u64(i.u64(1), i.u64(2), i.u64(3)));
        o.u32(svadclb_u32(i.u32(0), i.u32(1), i.u32(2)));
        o.u64(svadclt_u64(i.u64(1), i.u64(2), i.u64(3)));
        o.u32(svadclt_u32(i.u32(0), i.u32(1), i.u32(2)));
        o.s32(svaddhnb_s64(i.s64(0), i.s64(1)));
        o.s16(svaddhnb_s32(i.s32(1), i.s32(2)));
        o.s8(svaddhnb_s16(i.s16(1), i.s16(2)));
        o.s32(svaddhnt_s64(i.s32(0), i.s64(1), i.s64(2)));
        o.s16(svaddhnt_s32(i.s16(1), i.s32(2), i.s32(3)));
        o.s8(svaddhnt_s16(i.s8(1), i.s16(2), i.s16(3)));
        o.s8(svaddp_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svaddp_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svaddp_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svaddp_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.s8(svbcax_s8(i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svbcax_s64(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svbcax_s32(i.s32(0), i.s32(1), i.s32(2)));
        o.s16(svbcax_s16(i.s16(0), i.s16(1), i.s16(2)));
        o.s8(svbsl_s8(i.s8(0), i.s8(1), i.s8(2)));
        o.s64(svbsl_s64(i.s64(0), i.s64(1), i.s64(2)));
        o.s32(svbsl_s32(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svbsl_s16(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(svbsl1n_s8(i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svbsl1n_s64(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svbsl1n_s32(i.s32(0), i.s32(1), i.s32(2)));
        o.s16(svbsl1n_s16(i.s16(0), i.s16(1), i.s16(2)));
        o.s8(svbsl2n_s8(i.s8(0), i.s8(1), i.s8(2)));
        o.s64(svbsl2n_s64(i.s64(0), i.s64(1), i.s64(2)));
        o.s32(svbsl2n_s32(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svbsl2n_s16(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(svcadd_s8::<90>(i.s8(0), i.s8(1)));
        o.s8(svcadd_s8::<90>(i.s8(1), i.s8(2)));
        o.s64(svcadd_s64::<90>(i.s64(0), i.s64(1)));
        o.s64(svcadd_s64::<90>(i.s64(1), i.s64(2)));
        o.s32(svcadd_s32::<90>(i.s32(0), i.s32(1)));
        o.s32(svcadd_s32::<90>(i.s32(1), i.s32(2)));
        o.s16(svcadd_s16::<90>(i.s16(0), i.s16(1)));
        o.s16(svcadd_s16::<90>(i.s16(1), i.s16(2)));
        o.s64(svcdot_lane_s64::<0, 0>(i.s64(0), i.s16(1), i.s16(2)));
        o.s64(svcdot_lane_s64::<1, 0>(i.s64(1), i.s16(2), i.s16(3)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group32(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svcdot_lane_s32::<0, 0>(i.s32(0), i.s8(1), i.s8(2)));
        o.s32(svcdot_lane_s32::<3, 0>(i.s32(1), i.s8(2), i.s8(3)));
        o.s64(svcdot_s64::<0>(i.s64(0), i.s16(1), i.s16(2)));
        o.s64(svcdot_s64::<0>(i.s64(1), i.s16(2), i.s16(3)));
        o.s32(svcdot_s32::<0>(i.s32(0), i.s8(1), i.s8(2)));
        o.s32(svcdot_s32::<0>(i.s32(1), i.s8(2), i.s8(3)));
        o.s32(svcmla_lane_s32::<0, 0>(i.s32(0), i.s32(1), i.s32(2)));
        o.s32(svcmla_lane_s32::<1, 0>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svcmla_lane_s16::<0, 0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svcmla_lane_s16::<3, 0>(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(svcmla_s8::<0>(i.s8(0), i.s8(1), i.s8(2)));
        o.s8(svcmla_s8::<0>(i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svcmla_s64::<0>(i.s64(0), i.s64(1), i.s64(2)));
        o.s64(svcmla_s64::<0>(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svcmla_s32::<0>(i.s32(0), i.s32(1), i.s32(2)));
        o.s32(svcmla_s32::<0>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svcmla_s16::<0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svcmla_s16::<0>(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(sveor3_s8(i.s8(0), i.s8(1), i.s8(2)));
        o.s64(sveor3_s64(i.s64(0), i.s64(1), i.s64(2)));
        o.s32(sveor3_s32(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(sveor3_s16(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(sveorbt_s8(i.s8(1), i.s8(2), i.s8(3)));
        o.s64(sveorbt_s64(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(sveorbt_s32(i.s32(0), i.s32(1), i.s32(2)));
        o.s16(sveorbt_s16(i.s16(0), i.s16(1), i.s16(2)));
        o.s8(sveortb_s8(i.s8(1), i.s8(2), i.s8(3)));
        o.s64(sveortb_s64(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(sveortb_s32(i.s32(0), i.s32(1), i.s32(2)));
        o.s16(sveortb_s16(i.s16(0), i.s16(1), i.s16(2)));
        o.f64(svaddp_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svaddp_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.f64(svcvtlt_f64_f32_m(i.f64(1), i.p(2), i.f32(2)));
        o.f32(svcvtnt_f32_f64_m(i.f32(1), i.p(2), i.f64(2)));
        o.f32(svcvtx_f32_f64_m(i.f32(1), i.p(2), i.f64(2)));
        o.f32(svcvtxnt_f32_f64_m(i.f32(1), i.p(2), i.f64(2)));
        o.s64(svlogb_f64_m(i.s64(0), i.p(1), i.f64(1)));
        o.s32(svlogb_f32_m(i.s32(1), i.p(2), i.f32(2)));
        o.f64(svmaxnmp_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svmaxnmp_f32_m(i.p(0), i.f32(0), i.f32(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group33(i: &In, o: &mut Out) {
    unsafe {
        o.f64(svmaxp_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svmaxp_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svminnmp_f64_m(i.p(1), i.f64(1), i.f64(2)));
        o.f32(svminnmp_f32_m(i.p(0), i.f32(0), i.f32(1)));
        o.f64(svminp_f64_m(i.p(0), i.f64(0), i.f64(1)));
        o.f32(svminp_f32_m(i.p(1), i.f32(1), i.f32(2)));
        o.f64(svmul_lane_f64::<0>(i.f64(0), i.f64(1)));
        o.f64(svmul_lane_f64::<1>(i.f64(1), i.f64(2)));
        o.f32(svmul_lane_f32::<0>(i.f32(0), i.f32(1)));
        o.f32(svmul_lane_f32::<3>(i.f32(1), i.f32(2)));
        o.u64(svhistcnt_s64_z(i.p(1), i.s64(1), i.s64(2)));
        o.u32(svhistcnt_s32_z(i.p(0), i.s32(0), i.s32(1)));
        o.u8(svhistseg_s8(i.s8(0), i.s8(1)));
        o.f64(svldnt1_gather_s64index_f64(i.p(1), i.at::<f64>(64), i.os64(2)));
        o.s64(svldnt1sh_gather_s64index_s64(i.p(1), i.at::<i16>(64), i.os64(2)));
        o.s64(svldnt1sw_gather_s64index_s64(i.p(0), i.at::<i32>(64), i.os64(1)));
        o.s64(svldnt1_gather_s64index_s64(i.p(0), i.at::<i64>(64), i.os64(1)));
        o.f64(svldnt1_gather_s64offset_f64(i.p(0), i.at::<f64>(64), i.os64(1)));
        o.s64(svldnt1sh_gather_s64offset_s64(i.p(0), i.at::<i16>(64), i.os64(1)));
        o.s64(svldnt1sw_gather_s64offset_s64(i.p(1), i.at::<i32>(64), i.os64(2)));
        o.s64(svldnt1_gather_s64offset_s64(i.p(1), i.at::<i64>(64), i.os64(2)));
        o.s64(svldnt1sb_gather_s64offset_s64(i.p(0), i.at::<i8>(64), i.os64(1)));
        o.f64(svldnt1_gather_u64base_offset_f64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.s64(svldnt1sh_gather_u64base_offset_s64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.s64(svldnt1sw_gather_u64base_offset_s64(i.p(1), i.bu64(1, i.at::<u8>(1024)), -8));
        o.s64(svldnt1_gather_u64base_offset_s64(i.p(1), i.bu64(1, i.at::<u8>(1024)), -8));
        o.s64(svldnt1sb_gather_u64base_offset_s64(i.p(0), i.bu64(0, i.at::<u8>(1024)), 8));
        o.f32(svldnt1_gather_u32base_offset_f32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.s32(svldnt1sh_gather_u32base_offset_s32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.s32(svldnt1_gather_u32base_offset_s32(i.p(1), i.bu32(1), i.at::<u8>(1024) as i64));
        o.s32(svldnt1sb_gather_u32base_offset_s32(i.p(0), i.bu32(0), i.at::<u8>(1024) as i64));
        o.f32(svldnt1_gather_u32offset_f32(i.p(0), i.at::<f32>(64), i.ou32(1)));
        o.s32(svldnt1sh_gather_u32offset_s32(i.p(0), i.at::<i16>(64), i.ou32(1)));
        o.s32(svldnt1_gather_u32offset_s32(i.p(1), i.at::<i32>(64), i.ou32(2)));
        o.s32(svldnt1sb_gather_u32offset_s32(i.p(0), i.at::<i8>(64), i.ou32(1)));
        o.p(svmatch_s8(i.p(0), i.s8(0), i.s8(1)));
        o.p(svmatch_s16(i.p(1), i.s16(1), i.s16(2)));
        o.s64(svmla_lane_s64::<0>(i.s64(0), i.s64(1), i.s64(2)));
        o.s64(svmla_lane_s64::<1>(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svmla_lane_s32::<0>(i.s32(0), i.s32(1), i.s32(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group34(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svmla_lane_s32::<3>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svmla_lane_s16::<0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svmla_lane_s16::<7>(i.s16(1), i.s16(2), i.s16(3)));
        o.s64(svmls_lane_s64::<0>(i.s64(0), i.s64(1), i.s64(2)));
        o.s64(svmls_lane_s64::<1>(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svmls_lane_s32::<0>(i.s32(0), i.s32(1), i.s32(2)));
        o.s32(svmls_lane_s32::<3>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svmls_lane_s16::<0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svmls_lane_s16::<7>(i.s16(1), i.s16(2), i.s16(3)));
        o.s64(svmul_lane_s64::<0>(i.s64(0), i.s64(1)));
        o.s64(svmul_lane_s64::<1>(i.s64(1), i.s64(2)));
        o.s32(svmul_lane_s32::<0>(i.s32(0), i.s32(1)));
        o.s32(svmul_lane_s32::<3>(i.s32(1), i.s32(2)));
        o.s16(svmul_lane_s16::<0>(i.s16(0), i.s16(1)));
        o.s16(svmul_lane_s16::<7>(i.s16(1), i.s16(2)));
        o.s8(svnbsl_s8(i.s8(0), i.s8(1), i.s8(2)));
        o.s64(svnbsl_s64(i.s64(0), i.s64(1), i.s64(2)));
        o.s32(svnbsl_s32(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svnbsl_s16(i.s16(1), i.s16(2), i.s16(3)));
        o.p(svnmatch_s8(i.p(0), i.s8(0), i.s8(1)));
        o.p(svnmatch_s16(i.p(1), i.s16(1), i.s16(2)));
        o.u8(svpmul_u8(i.u8(1), i.u8(2)));
        o.p(svunpkhi_b(i.p(1)));
        o.p(svunpklo_b(i.p(1)));
        o.s32(svraddhnb_s64(i.s64(0), i.s64(1)));
        o.s16(svraddhnb_s32(i.s32(1), i.s32(2)));
        o.s8(svraddhnb_s16(i.s16(1), i.s16(2)));
        o.s32(svraddhnt_s64(i.s32(0), i.s64(1), i.s64(2)));
        o.s16(svraddhnt_s32(i.s16(1), i.s32(2), i.s32(3)));
        o.s8(svraddhnt_s16(i.s8(1), i.s16(2), i.s16(3)));
        o.s32(svrshrnb_n_s64::<1>(i.s64(0)));
        o.s32(svrshrnb_n_s64::<32>(i.s64(1)));
        o.s16(svrshrnb_n_s32::<1>(i.s32(0)));
        o.s16(svrshrnb_n_s32::<16>(i.s32(1)));
        o.s8(svrshrnb_n_s16::<1>(i.s16(0)));
        o.s8(svrshrnb_n_s16::<8>(i.s16(1)));
        o.s32(svrshrnt_n_s64::<1>(i.s32(0), i.s64(1)));
        o.s32(svrshrnt_n_s64::<32>(i.s32(1), i.s64(2)));
        o.s16(svrshrnt_n_s32::<1>(i.s16(0), i.s32(1)));
        o.s16(svrshrnt_n_s32::<16>(i.s16(1), i.s32(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group35(i: &In, o: &mut Out) {
    unsafe {
        o.s8(svrshrnt_n_s16::<1>(i.s8(0), i.s16(1)));
        o.s8(svrshrnt_n_s16::<8>(i.s8(1), i.s16(2)));
        o.s32(svrsubhnb_s64(i.s64(1), i.s64(2)));
        o.s16(svrsubhnb_s32(i.s32(0), i.s32(1)));
        o.s8(svrsubhnb_s16(i.s16(0), i.s16(1)));
        o.s32(svrsubhnt_s64(i.s32(1), i.s64(2), i.s64(3)));
        o.s16(svrsubhnt_s32(i.s16(0), i.s32(1), i.s32(2)));
        o.s8(svrsubhnt_s16(i.s8(0), i.s16(1), i.s16(2)));
        o.s8(svaba_s8(i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svaba_s64(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svaba_s32(i.s32(0), i.s32(1), i.s32(2)));
        o.s16(svaba_s16(i.s16(0), i.s16(1), i.s16(2)));
        o.s64(svabalb_s64(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svabalb_s32(i.s32(0), i.s16(1), i.s16(2)));
        o.s16(svabalb_s16(i.s16(0), i.s8(1), i.s8(2)));
        o.s64(svabalt_s64(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svabalt_s32(i.s32(0), i.s16(1), i.s16(2)));
        o.s16(svabalt_s16(i.s16(0), i.s8(1), i.s8(2)));
        o.s64(svabdlb_s64(i.s32(0), i.s32(1)));
        o.s32(svabdlb_s32(i.s16(1), i.s16(2)));
        o.s16(svabdlb_s16(i.s8(1), i.s8(2)));
        o.s64(svabdlt_s64(i.s32(0), i.s32(1)));
        o.s32(svabdlt_s32(i.s16(1), i.s16(2)));
        o.s16(svabdlt_s16(i.s8(1), i.s8(2)));
        o.s64(svadalp_s64_m(i.p(1), i.s64(1), i.s32(2)));
        o.s32(svadalp_s32_m(i.p(0), i.s32(0), i.s16(1)));
        o.s16(svadalp_s16_m(i.p(0), i.s16(0), i.s8(1)));
        o.s64(svaddlb_s64(i.s32(0), i.s32(1)));
        o.s32(svaddlb_s32(i.s16(1), i.s16(2)));
        o.s16(svaddlb_s16(i.s8(1), i.s8(2)));
        o.s64(svaddlbt_s64(i.s32(0), i.s32(1)));
        o.s32(svaddlbt_s32(i.s16(1), i.s16(2)));
        o.s16(svaddlbt_s16(i.s8(1), i.s8(2)));
        o.s64(svaddlt_s64(i.s32(0), i.s32(1)));
        o.s32(svaddlt_s32(i.s16(1), i.s16(2)));
        o.s16(svaddlt_s16(i.s8(1), i.s8(2)));
        o.s64(svaddwb_s64(i.s64(1), i.s32(2)));
        o.s32(svaddwb_s32(i.s32(0), i.s16(1)));
        o.s16(svaddwb_s16(i.s16(0), i.s8(1)));
        o.s64(svaddwt_s64(i.s64(1), i.s32(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group36(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svaddwt_s32(i.s32(0), i.s16(1)));
        o.s16(svaddwt_s16(i.s16(0), i.s8(1)));
        o.u64(svsbclb_u64(i.u64(1), i.u64(2), i.u64(3)));
        o.u32(svsbclb_u32(i.u32(0), i.u32(1), i.u32(2)));
        o.u64(svsbclt_u64(i.u64(1), i.u64(2), i.u64(3)));
        o.u32(svsbclt_u32(i.u32(0), i.u32(1), i.u32(2)));
        o.s8(svhadd_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svhadd_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svhadd_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svhadd_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.s32(svshrnb_n_s64::<1>(i.s64(0)));
        o.s32(svshrnb_n_s64::<32>(i.s64(1)));
        o.s16(svshrnb_n_s32::<1>(i.s32(0)));
        o.s16(svshrnb_n_s32::<16>(i.s32(1)));
        o.s8(svshrnb_n_s16::<1>(i.s16(0)));
        o.s8(svshrnb_n_s16::<8>(i.s16(1)));
        o.s32(svshrnt_n_s64::<1>(i.s32(0), i.s64(1)));
        o.s32(svshrnt_n_s64::<32>(i.s32(1), i.s64(2)));
        o.s16(svshrnt_n_s32::<1>(i.s16(0), i.s32(1)));
        o.s16(svshrnt_n_s32::<16>(i.s16(1), i.s32(2)));
        o.s8(svshrnt_n_s16::<1>(i.s8(0), i.s16(1)));
        o.s8(svshrnt_n_s16::<8>(i.s8(1), i.s16(2)));
        o.s8(svhsub_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svhsub_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svhsub_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svhsub_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s8(svhsubr_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svhsubr_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svhsubr_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svhsubr_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s8(svsli_n_s8::<0>(i.s8(0), i.s8(1)));
        o.s8(svsli_n_s8::<7>(i.s8(1), i.s8(2)));
        o.s64(svsli_n_s64::<0>(i.s64(0), i.s64(1)));
        o.s64(svsli_n_s64::<63>(i.s64(1), i.s64(2)));
        o.s32(svsli_n_s32::<0>(i.s32(0), i.s32(1)));
        o.s32(svsli_n_s32::<31>(i.s32(1), i.s32(2)));
        o.s16(svsli_n_s16::<0>(i.s16(0), i.s16(1)));
        o.s16(svsli_n_s16::<15>(i.s16(1), i.s16(2)));
        o.s8(svmaxp_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svmaxp_s64_m(i.p(1), i.s64(1), i.s64(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group37(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svmaxp_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svmaxp_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s8(svminp_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svminp_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svminp_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svminp_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s64(svmlalb_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svmlalb_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlalb_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svmlalb_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svmlalb_s64(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlalb_s32(i.s32(0), i.s16(1), i.s16(2)));
        o.s16(svmlalb_s16(i.s16(0), i.s8(1), i.s8(2)));
        o.s64(svmlalt_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svmlalt_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlalt_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svmlalt_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svmlalt_s64(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlalt_s32(i.s32(0), i.s16(1), i.s16(2)));
        o.s16(svmlalt_s16(i.s16(0), i.s8(1), i.s8(2)));
        o.s64(svmlslb_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svmlslb_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlslb_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svmlslb_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svmlslb_s64(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlslb_s32(i.s32(0), i.s16(1), i.s16(2)));
        o.s16(svmlslb_s16(i.s16(0), i.s8(1), i.s8(2)));
        o.s64(svmlslt_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svmlslt_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlslt_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svmlslt_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svmlslt_s64(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svmlslt_s32(i.s32(0), i.s16(1), i.s16(2)));
        o.s16(svmlslt_s16(i.s16(0), i.s8(1), i.s8(2)));
        o.s64(svmullb_lane_s64::<0>(i.s32(0), i.s32(1)));
        o.s64(svmullb_lane_s64::<3>(i.s32(1), i.s32(2)));
        o.s32(svmullb_lane_s32::<0>(i.s16(0), i.s16(1)));
        o.s32(svmullb_lane_s32::<7>(i.s16(1), i.s16(2)));
        o.s64(svmullb_s64(i.s32(1), i.s32(2)));
        o.s32(svmullb_s32(i.s16(0), i.s16(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group38(i: &In, o: &mut Out) {
    unsafe {
        o.s16(svmullb_s16(i.s8(0), i.s8(1)));
        o.s64(svmullt_lane_s64::<0>(i.s32(0), i.s32(1)));
        o.s64(svmullt_lane_s64::<3>(i.s32(1), i.s32(2)));
        o.s32(svmullt_lane_s32::<0>(i.s16(0), i.s16(1)));
        o.s32(svmullt_lane_s32::<7>(i.s16(1), i.s16(2)));
        o.s64(svmullt_s64(i.s32(1), i.s32(2)));
        o.s32(svmullt_s32(i.s16(0), i.s16(1)));
        o.s16(svmullt_s16(i.s8(0), i.s8(1)));
        o.s8(svqabs_s8_m(i.s8(0), i.p(1), i.s8(1)));
        o.s64(svqabs_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svqabs_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s16(svqabs_s16_m(i.s16(1), i.p(2), i.s16(2)));
        o.s8(svqadd_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svqadd_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svqadd_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svqadd_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s8(svqcadd_s8::<90>(i.s8(0), i.s8(1)));
        o.s8(svqcadd_s8::<90>(i.s8(1), i.s8(2)));
        o.s64(svqcadd_s64::<90>(i.s64(0), i.s64(1)));
        o.s64(svqcadd_s64::<90>(i.s64(1), i.s64(2)));
        o.s32(svqcadd_s32::<90>(i.s32(0), i.s32(1)));
        o.s32(svqcadd_s32::<90>(i.s32(1), i.s32(2)));
        o.s16(svqcadd_s16::<90>(i.s16(0), i.s16(1)));
        o.s16(svqcadd_s16::<90>(i.s16(1), i.s16(2)));
        o.s64(svqdmlalb_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svqdmlalb_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svqdmlalb_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svqdmlalb_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svqdmlalb_s64(i.s64(0), i.s32(1), i.s32(2)));
        o.s32(svqdmlalb_s32(i.s32(1), i.s16(2), i.s16(3)));
        o.s16(svqdmlalb_s16(i.s16(1), i.s8(2), i.s8(3)));
        o.s64(svqdmlalbt_s64(i.s64(0), i.s32(1), i.s32(2)));
        o.s32(svqdmlalbt_s32(i.s32(1), i.s16(2), i.s16(3)));
        o.s16(svqdmlalbt_s16(i.s16(1), i.s8(2), i.s8(3)));
        o.s64(svqdmlalt_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svqdmlalt_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svqdmlalt_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svqdmlalt_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svqdmlalt_s64(i.s64(0), i.s32(1), i.s32(2)));
        o.s32(svqdmlalt_s32(i.s32(1), i.s16(2), i.s16(3)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group39(i: &In, o: &mut Out) {
    unsafe {
        o.s16(svqdmlalt_s16(i.s16(1), i.s8(2), i.s8(3)));
        o.s64(svqdmlslb_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svqdmlslb_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svqdmlslb_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svqdmlslb_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svqdmlslb_s64(i.s64(0), i.s32(1), i.s32(2)));
        o.s32(svqdmlslb_s32(i.s32(1), i.s16(2), i.s16(3)));
        o.s16(svqdmlslb_s16(i.s16(1), i.s8(2), i.s8(3)));
        o.s64(svqdmlslbt_s64(i.s64(0), i.s32(1), i.s32(2)));
        o.s32(svqdmlslbt_s32(i.s32(1), i.s16(2), i.s16(3)));
        o.s16(svqdmlslbt_s16(i.s16(1), i.s8(2), i.s8(3)));
        o.s64(svqdmlslt_lane_s64::<0>(i.s64(0), i.s32(1), i.s32(2)));
        o.s64(svqdmlslt_lane_s64::<3>(i.s64(1), i.s32(2), i.s32(3)));
        o.s32(svqdmlslt_lane_s32::<0>(i.s32(0), i.s16(1), i.s16(2)));
        o.s32(svqdmlslt_lane_s32::<7>(i.s32(1), i.s16(2), i.s16(3)));
        o.s64(svqdmlslt_s64(i.s64(0), i.s32(1), i.s32(2)));
        o.s32(svqdmlslt_s32(i.s32(1), i.s16(2), i.s16(3)));
        o.s16(svqdmlslt_s16(i.s16(1), i.s8(2), i.s8(3)));
        o.s64(svqdmulh_lane_s64::<0>(i.s64(0), i.s64(1)));
        o.s64(svqdmulh_lane_s64::<1>(i.s64(1), i.s64(2)));
        o.s32(svqdmulh_lane_s32::<0>(i.s32(0), i.s32(1)));
        o.s32(svqdmulh_lane_s32::<3>(i.s32(1), i.s32(2)));
        o.s16(svqdmulh_lane_s16::<0>(i.s16(0), i.s16(1)));
        o.s16(svqdmulh_lane_s16::<7>(i.s16(1), i.s16(2)));
        o.s8(svqdmulh_s8(i.s8(0), i.s8(1)));
        o.s64(svqdmulh_s64(i.s64(0), i.s64(1)));
        o.s32(svqdmulh_s32(i.s32(1), i.s32(2)));
        o.s16(svqdmulh_s16(i.s16(1), i.s16(2)));
        o.s64(svqdmullb_lane_s64::<0>(i.s32(0), i.s32(1)));
        o.s64(svqdmullb_lane_s64::<3>(i.s32(1), i.s32(2)));
        o.s32(svqdmullb_lane_s32::<0>(i.s16(0), i.s16(1)));
        o.s32(svqdmullb_lane_s32::<7>(i.s16(1), i.s16(2)));
        o.s64(svqdmullb_s64(i.s32(0), i.s32(1)));
        o.s32(svqdmullb_s32(i.s16(1), i.s16(2)));
        o.s16(svqdmullb_s16(i.s8(1), i.s8(2)));
        o.s64(svqdmullt_lane_s64::<0>(i.s32(0), i.s32(1)));
        o.s64(svqdmullt_lane_s64::<3>(i.s32(1), i.s32(2)));
        o.s32(svqdmullt_lane_s32::<0>(i.s16(0), i.s16(1)));
        o.s32(svqdmullt_lane_s32::<7>(i.s16(1), i.s16(2)));
        o.s64(svqdmullt_s64(i.s32(0), i.s32(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group40(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svqdmullt_s32(i.s16(1), i.s16(2)));
        o.s16(svqdmullt_s16(i.s8(1), i.s8(2)));
        o.s8(svqneg_s8_m(i.s8(0), i.p(1), i.s8(1)));
        o.s64(svqneg_s64_m(i.s64(0), i.p(1), i.s64(1)));
        o.s32(svqneg_s32_m(i.s32(1), i.p(2), i.s32(2)));
        o.s16(svqneg_s16_m(i.s16(1), i.p(2), i.s16(2)));
        o.s32(svqrdcmlah_lane_s32::<0, 0>(i.s32(0), i.s32(1), i.s32(2)));
        o.s32(svqrdcmlah_lane_s32::<1, 0>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svqrdcmlah_lane_s16::<0, 0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svqrdcmlah_lane_s16::<3, 0>(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(svqrdcmlah_s8::<0>(i.s8(0), i.s8(1), i.s8(2)));
        o.s8(svqrdcmlah_s8::<0>(i.s8(1), i.s8(2), i.s8(3)));
        o.s64(svqrdcmlah_s64::<0>(i.s64(0), i.s64(1), i.s64(2)));
        o.s64(svqrdcmlah_s64::<0>(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svqrdcmlah_s32::<0>(i.s32(0), i.s32(1), i.s32(2)));
        o.s32(svqrdcmlah_s32::<0>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svqrdcmlah_s16::<0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svqrdcmlah_s16::<0>(i.s16(1), i.s16(2), i.s16(3)));
        o.s64(svqrdmlah_lane_s64::<0>(i.s64(0), i.s64(1), i.s64(2)));
        o.s64(svqrdmlah_lane_s64::<1>(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svqrdmlah_lane_s32::<0>(i.s32(0), i.s32(1), i.s32(2)));
        o.s32(svqrdmlah_lane_s32::<3>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svqrdmlah_lane_s16::<0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svqrdmlah_lane_s16::<7>(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(svqrdmlah_s8(i.s8(0), i.s8(1), i.s8(2)));
        o.s64(svqrdmlah_s64(i.s64(0), i.s64(1), i.s64(2)));
        o.s32(svqrdmlah_s32(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svqrdmlah_s16(i.s16(1), i.s16(2), i.s16(3)));
        o.s64(svqrdmlsh_lane_s64::<0>(i.s64(0), i.s64(1), i.s64(2)));
        o.s64(svqrdmlsh_lane_s64::<1>(i.s64(1), i.s64(2), i.s64(3)));
        o.s32(svqrdmlsh_lane_s32::<0>(i.s32(0), i.s32(1), i.s32(2)));
        o.s32(svqrdmlsh_lane_s32::<3>(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svqrdmlsh_lane_s16::<0>(i.s16(0), i.s16(1), i.s16(2)));
        o.s16(svqrdmlsh_lane_s16::<7>(i.s16(1), i.s16(2), i.s16(3)));
        o.s8(svqrdmlsh_s8(i.s8(0), i.s8(1), i.s8(2)));
        o.s64(svqrdmlsh_s64(i.s64(0), i.s64(1), i.s64(2)));
        o.s32(svqrdmlsh_s32(i.s32(1), i.s32(2), i.s32(3)));
        o.s16(svqrdmlsh_s16(i.s16(1), i.s16(2), i.s16(3)));
        o.s64(svqrdmulh_lane_s64::<0>(i.s64(0), i.s64(1)));
        o.s64(svqrdmulh_lane_s64::<1>(i.s64(1), i.s64(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group41(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svqrdmulh_lane_s32::<0>(i.s32(0), i.s32(1)));
        o.s32(svqrdmulh_lane_s32::<3>(i.s32(1), i.s32(2)));
        o.s16(svqrdmulh_lane_s16::<0>(i.s16(0), i.s16(1)));
        o.s16(svqrdmulh_lane_s16::<7>(i.s16(1), i.s16(2)));
        o.s8(svqrdmulh_s8(i.s8(0), i.s8(1)));
        o.s64(svqrdmulh_s64(i.s64(0), i.s64(1)));
        o.s32(svqrdmulh_s32(i.s32(1), i.s32(2)));
        o.s16(svqrdmulh_s16(i.s16(1), i.s16(2)));
        o.s8(svqrshl_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svqrshl_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svqrshl_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svqrshl_s16_m(i.p(0), i.s16(0), i.s16(1)));
        o.s32(svqrshrnb_n_s64::<1>(i.s64(0)));
        o.s32(svqrshrnb_n_s64::<32>(i.s64(1)));
        o.s16(svqrshrnb_n_s32::<1>(i.s32(0)));
        o.s16(svqrshrnb_n_s32::<16>(i.s32(1)));
        o.s8(svqrshrnb_n_s16::<1>(i.s16(0)));
        o.s8(svqrshrnb_n_s16::<8>(i.s16(1)));
        o.s32(svqrshrnt_n_s64::<1>(i.s32(0), i.s64(1)));
        o.s32(svqrshrnt_n_s64::<32>(i.s32(1), i.s64(2)));
        o.s16(svqrshrnt_n_s32::<1>(i.s16(0), i.s32(1)));
        o.s16(svqrshrnt_n_s32::<16>(i.s16(1), i.s32(2)));
        o.s8(svqrshrnt_n_s16::<1>(i.s8(0), i.s16(1)));
        o.s8(svqrshrnt_n_s16::<8>(i.s8(1), i.s16(2)));
        o.u32(svqrshrunb_n_s64::<1>(i.s64(0)));
        o.u32(svqrshrunb_n_s64::<32>(i.s64(1)));
        o.u16(svqrshrunb_n_s32::<1>(i.s32(0)));
        o.u16(svqrshrunb_n_s32::<16>(i.s32(1)));
        o.u8(svqrshrunb_n_s16::<1>(i.s16(0)));
        o.u8(svqrshrunb_n_s16::<8>(i.s16(1)));
        o.u32(svqrshrunt_n_s64::<1>(i.u32(0), i.s64(1)));
        o.u32(svqrshrunt_n_s64::<32>(i.u32(1), i.s64(2)));
        o.u16(svqrshrunt_n_s32::<1>(i.u16(0), i.s32(1)));
        o.u16(svqrshrunt_n_s32::<16>(i.u16(1), i.s32(2)));
        o.u8(svqrshrunt_n_s16::<1>(i.u8(0), i.s16(1)));
        o.u8(svqrshrunt_n_s16::<8>(i.u8(1), i.s16(2)));
        o.s8(svqshl_s8_m(i.p(1), i.s8(1), i.s8(2)));
        o.s64(svqshl_s64_m(i.p(1), i.s64(1), i.s64(2)));
        o.s32(svqshl_s32_m(i.p(0), i.s32(0), i.s32(1)));
        o.s16(svqshl_s16_m(i.p(0), i.s16(0), i.s16(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group42(i: &In, o: &mut Out) {
    unsafe {
        o.u8(svqshlu_n_s8_m::<0>(i.p(0), i.s8(0)));
        o.u8(svqshlu_n_s8_m::<7>(i.p(1), i.s8(1)));
        o.u64(svqshlu_n_s64_m::<0>(i.p(0), i.s64(0)));
        o.u64(svqshlu_n_s64_m::<63>(i.p(1), i.s64(1)));
        o.u32(svqshlu_n_s32_m::<0>(i.p(0), i.s32(0)));
        o.u32(svqshlu_n_s32_m::<31>(i.p(1), i.s32(1)));
        o.u16(svqshlu_n_s16_m::<0>(i.p(0), i.s16(0)));
        o.u16(svqshlu_n_s16_m::<15>(i.p(1), i.s16(1)));
        o.s32(svqshrnb_n_s64::<1>(i.s64(0)));
        o.s32(svqshrnb_n_s64::<32>(i.s64(1)));
        o.s16(svqshrnb_n_s32::<1>(i.s32(0)));
        o.s16(svqshrnb_n_s32::<16>(i.s32(1)));
        o.s8(svqshrnb_n_s16::<1>(i.s16(0)));
        o.s8(svqshrnb_n_s16::<8>(i.s16(1)));
        o.s32(svqshrnt_n_s64::<1>(i.s32(0), i.s64(1)));
        o.s32(svqshrnt_n_s64::<32>(i.s32(1), i.s64(2)));
        o.s16(svqshrnt_n_s32::<1>(i.s16(0), i.s32(1)));
        o.s16(svqshrnt_n_s32::<16>(i.s16(1), i.s32(2)));
        o.s8(svqshrnt_n_s16::<1>(i.s8(0), i.s16(1)));
        o.s8(svqshrnt_n_s16::<8>(i.s8(1), i.s16(2)));
        o.u32(svqshrunb_n_s64::<1>(i.s64(0)));
        o.u32(svqshrunb_n_s64::<32>(i.s64(1)));
        o.u16(svqshrunb_n_s32::<1>(i.s32(0)));
        o.u16(svqshrunb_n_s32::<16>(i.s32(1)));
        o.u8(svqshrunb_n_s16::<1>(i.s16(0)));
        o.u8(svqshrunb_n_s16::<8>(i.s16(1)));
        o.u32(svqshrunt_n_s64::<1>(i.u32(0), i.s64(1)));
        o.u32(svqshrunt_n_s64::<32>(i.u32(1), i.s64(2)));
        o.u16(svqshrunt_n_s32::<1>(i.u16(0), i.s32(1)));
        o.u16(svqshrunt_n_s32::<16>(i.u16(1), i.s32(2)));
        o.u8(svqshrunt_n_s16::<1>(i.u8(0), i.s16(1)));
        o.u8(svqshrunt_n_s16::<8>(i.u8(1), i.s16(2)));
        o.s8(svqsub_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svqsub_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svqsub_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svqsub_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.s8(svqsubr_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svqsubr_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svqsubr_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svqsubr_s16_m(i.p(1), i.s16(1), i.s16(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group43(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svqxtnb_s64(i.s64(0)));
        o.s16(svqxtnb_s32(i.s32(1)));
        o.s8(svqxtnb_s16(i.s16(1)));
        o.s32(svqxtnt_s64(i.s32(0), i.s64(1)));
        o.s16(svqxtnt_s32(i.s16(1), i.s32(2)));
        o.s8(svqxtnt_s16(i.s8(1), i.s16(2)));
        o.u32(svqxtunb_s64(i.s64(1)));
        o.u16(svqxtunb_s32(i.s32(0)));
        o.u8(svqxtunb_s16(i.s16(0)));
        o.u32(svqxtunt_s64(i.u32(1), i.s64(2)));
        o.u16(svqxtunt_s32(i.u16(0), i.s32(1)));
        o.u8(svqxtunt_s16(i.u8(0), i.s16(1)));
        o.s8(svrhadd_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svrhadd_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svrhadd_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svrhadd_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.s8(svsri_n_s8::<1>(i.s8(0), i.s8(1)));
        o.s8(svsri_n_s8::<8>(i.s8(1), i.s8(2)));
        o.s64(svsri_n_s64::<1>(i.s64(0), i.s64(1)));
        o.s64(svsri_n_s64::<64>(i.s64(1), i.s64(2)));
        o.s32(svsri_n_s32::<1>(i.s32(0), i.s32(1)));
        o.s32(svsri_n_s32::<32>(i.s32(1), i.s32(2)));
        o.s16(svsri_n_s16::<1>(i.s16(0), i.s16(1)));
        o.s16(svsri_n_s16::<16>(i.s16(1), i.s16(2)));
        o.s8(svrshl_s8_m(i.p(0), i.s8(0), i.s8(1)));
        o.s64(svrshl_s64_m(i.p(0), i.s64(0), i.s64(1)));
        o.s32(svrshl_s32_m(i.p(1), i.s32(1), i.s32(2)));
        o.s16(svrshl_s16_m(i.p(1), i.s16(1), i.s16(2)));
        o.s8(svrshr_n_s8_m::<1>(i.p(0), i.s8(0)));
        o.s8(svrshr_n_s8_m::<8>(i.p(1), i.s8(1)));
        o.s64(svrshr_n_s64_m::<1>(i.p(0), i.s64(0)));
        o.s64(svrshr_n_s64_m::<64>(i.p(1), i.s64(1)));
        o.s32(svrshr_n_s32_m::<1>(i.p(0), i.s32(0)));
        o.s32(svrshr_n_s32_m::<32>(i.p(1), i.s32(1)));
        o.s16(svrshr_n_s16_m::<1>(i.p(0), i.s16(0)));
        o.s16(svrshr_n_s16_m::<16>(i.p(1), i.s16(1)));
        o.s8(svrsra_n_s8::<1>(i.s8(0), i.s8(1)));
        o.s8(svrsra_n_s8::<8>(i.s8(1), i.s8(2)));
        o.s64(svrsra_n_s64::<1>(i.s64(0), i.s64(1)));
        o.s64(svrsra_n_s64::<64>(i.s64(1), i.s64(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group44(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svrsra_n_s32::<1>(i.s32(0), i.s32(1)));
        o.s32(svrsra_n_s32::<32>(i.s32(1), i.s32(2)));
        o.s16(svrsra_n_s16::<1>(i.s16(0), i.s16(1)));
        o.s16(svrsra_n_s16::<16>(i.s16(1), i.s16(2)));
        o.s64(svshllb_n_s64::<0>(i.s32(0)));
        o.s64(svshllb_n_s64::<31>(i.s32(1)));
        o.s32(svshllb_n_s32::<0>(i.s16(0)));
        o.s32(svshllb_n_s32::<15>(i.s16(1)));
        o.s16(svshllb_n_s16::<0>(i.s8(0)));
        o.s16(svshllb_n_s16::<7>(i.s8(1)));
        o.s64(svshllt_n_s64::<0>(i.s32(0)));
        o.s64(svshllt_n_s64::<31>(i.s32(1)));
        o.s32(svshllt_n_s32::<0>(i.s16(0)));
        o.s32(svshllt_n_s32::<15>(i.s16(1)));
        o.s16(svshllt_n_s16::<0>(i.s8(0)));
        o.s16(svshllt_n_s16::<7>(i.s8(1)));
        o.s8(svsra_n_s8::<1>(i.s8(0), i.s8(1)));
        o.s8(svsra_n_s8::<8>(i.s8(1), i.s8(2)));
        o.s64(svsra_n_s64::<1>(i.s64(0), i.s64(1)));
        o.s64(svsra_n_s64::<64>(i.s64(1), i.s64(2)));
        o.s32(svsra_n_s32::<1>(i.s32(0), i.s32(1)));
        o.s32(svsra_n_s32::<32>(i.s32(1), i.s32(2)));
        o.s16(svsra_n_s16::<1>(i.s16(0), i.s16(1)));
        o.s16(svsra_n_s16::<16>(i.s16(1), i.s16(2)));
        o.s64(svsublb_s64(i.s32(1), i.s32(2)));
        o.s32(svsublb_s32(i.s16(0), i.s16(1)));
        o.s16(svsublb_s16(i.s8(0), i.s8(1)));
        o.s64(svsublbt_s64(i.s32(1), i.s32(2)));
        o.s32(svsublbt_s32(i.s16(0), i.s16(1)));
        o.s16(svsublbt_s16(i.s8(0), i.s8(1)));
        o.s64(svsublt_s64(i.s32(1), i.s32(2)));
        o.s32(svsublt_s32(i.s16(0), i.s16(1)));
        o.s16(svsublt_s16(i.s8(0), i.s8(1)));
        o.s64(svsubltb_s64(i.s32(1), i.s32(2)));
        o.s32(svsubltb_s32(i.s16(0), i.s16(1)));
        o.s16(svsubltb_s16(i.s8(0), i.s8(1)));
        o.s64(svsubwb_s64(i.s64(0), i.s32(1)));
        o.s32(svsubwb_s32(i.s32(1), i.s16(2)));
        o.s16(svsubwb_s16(i.s16(1), i.s8(2)));
        o.s64(svsubwt_s64(i.s64(0), i.s32(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group45(i: &In, o: &mut Out) {
    unsafe {
        o.s32(svsubwt_s32(i.s32(1), i.s16(2)));
        o.s16(svsubwt_s16(i.s16(1), i.s8(2)));
        svstnt1_scatter_s64index_f64(i.p(1), o.at::<f64>(64), i.os64(2), i.f64(1));
        o.written();
        svstnt1h_scatter_s64index_s64(i.p(0), o.at::<i16>(64), i.os64(1), i.s64(0));
        o.written();
        svstnt1w_scatter_s64index_s64(i.p(1), o.at::<i32>(64), i.os64(2), i.s64(1));
        o.written();
        svstnt1_scatter_s64index_s64(i.p(0), o.at::<i64>(64), i.os64(1), i.s64(0));
        o.written();
        svstnt1_scatter_s64offset_f64(i.p(0), o.at::<f64>(64), i.os64(1), i.f64(0));
        o.written();
        svstnt1h_scatter_s64offset_s64(i.p(1), o.at::<i16>(64), i.os64(2), i.s64(1));
        o.written();
        svstnt1w_scatter_s64offset_s64(i.p(0), o.at::<i32>(64), i.os64(1), i.s64(0));
        o.written();
        svstnt1_scatter_s64offset_s64(i.p(1), o.at::<i64>(64), i.os64(2), i.s64(1));
        o.written();
        svstnt1b_scatter_s64offset_s64(i.p(1), o.at::<i8>(64), i.os64(2), i.s64(1));
        o.written();
        svstnt1_scatter_u64base_offset_f64(i.p(0), i.bu64(0, o.at::<u8>(1024)), 8, i.f64(0));
        o.written();
        svstnt1h_scatter_u64base_offset_s64(i.p(1), i.bu64(1, o.at::<u8>(1024)), -8, i.s64(1));
        o.written();
        svstnt1w_scatter_u64base_offset_s64(i.p(0), i.bu64(0, o.at::<u8>(1024)), 8, i.s64(0));
        o.written();
        svstnt1_scatter_u64base_offset_s64(i.p(1), i.bu64(1, o.at::<u8>(1024)), -8, i.s64(1));
        o.written();
        svstnt1b_scatter_u64base_offset_s64(i.p(1), i.bu64(1, o.at::<u8>(1024)), -8, i.s64(1));
        o.written();
        svstnt1_scatter_u32base_offset_f32(i.p(0), i.bu32(0), o.at::<u8>(1024) as i64, i.f32(0));
        o.written();
        svstnt1h_scatter_u32base_offset_s32(i.p(1), i.bu32(1), o.at::<u8>(1024) as i64, i.s32(1));
        o.written();
        svstnt1_scatter_u32base_offset_s32(i.p(1), i.bu32(1), o.at::<u8>(1024) as i64, i.s32(1));
        o.written();
        svstnt1b_scatter_u32base_offset_s32(i.p(1), i.bu32(1), o.at::<u8>(1024) as i64, i.s32(1));
        o.written();
        svstnt1_scatter_u32offset_f32(i.p(0), o.at::<f32>(64), i.ou32(1), i.f32(0));
        o.written();
        svstnt1h_scatter_u32offset_s32(i.p(1), o.at::<i16>(64), i.ou32(2), i.s32(1));
        o.written();
        svstnt1_scatter_u32offset_s32(i.p(1), o.at::<i32>(64), i.ou32(2), i.s32(1));
        o.written();
        svstnt1b_scatter_u32offset_s32(i.p(1), o.at::<i8>(64), i.ou32(2), i.s32(1));
        o.written();
        o.s32(svsubhnb_s64(i.s64(1), i.s64(2)));
        o.s16(svsubhnb_s32(i.s32(0), i.s32(1)));
        o.s8(svsubhnb_s16(i.s16(0), i.s16(1)));
        o.s32(svsubhnt_s64(i.s32(1), i.s64(2), i.s64(3)));
        o.s16(svsubhnt_s32(i.s16(0), i.s32(1), i.s32(2)));
        o.s8(svsubhnt_s16(i.s8(0), i.s16(1), i.s16(2)));
        o.s64(svunpkhi_s64(i.s32(0)));
        o.s32(svunpkhi_s32(i.s16(1)));
        o.s16(svunpkhi_s16(i.s8(1)));
        o.s64(svunpklo_s64(i.s32(0)));
        o.s32(svunpklo_s32(i.s16(1)));
        o.s16(svunpklo_s16(i.s8(1)));
        o.s8(svuqadd_s8_m(i.p(0), i.s8(0), i.u8(1)));
        o.s64(svuqadd_s64_m(i.p(0), i.s64(0), i.u64(1)));
        o.s32(svuqadd_s32_m(i.p(1), i.s32(1), i.u32(2)));
        o.s16(svuqadd_s16_m(i.p(1), i.s16(1), i.u16(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group46(i: &In, o: &mut Out) {
    unsafe {
        o.s8(svtbl2_s8(i.s8x2(1), i.u8(2)));
        o.f64(svtbl2_f64(i.f64x2(0), i.u64(1)));
        o.s64(svtbl2_s64(i.s64x2(1), i.u64(2)));
        o.f32(svtbl2_f32(i.f32x2(1), i.u32(2)));
        o.s32(svtbl2_s32(i.s32x2(0), i.u32(1)));
        o.s16(svtbl2_s16(i.s16x2(0), i.u16(1)));
        o.s8(svtbx_s8(i.s8(1), i.s8(2), i.u8(3)));
        o.f64(svtbx_f64(i.f64(0), i.f64(1), i.u64(2)));
        o.s64(svtbx_s64(i.s64(1), i.s64(2), i.u64(3)));
        o.f32(svtbx_f32(i.f32(1), i.f32(2), i.u32(3)));
        o.s32(svtbx_s32(i.s32(0), i.s32(1), i.u32(2)));
        o.s16(svtbx_s16(i.s16(0), i.s16(1), i.u16(2)));
        o.u8(svaba_u8(i.u8(1), i.u8(2), i.u8(3)));
        o.u64(svaba_u64(i.u64(1), i.u64(2), i.u64(3)));
        o.u32(svaba_u32(i.u32(0), i.u32(1), i.u32(2)));
        o.u16(svaba_u16(i.u16(0), i.u16(1), i.u16(2)));
        o.u64(svabalb_u64(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svabalb_u32(i.u32(0), i.u16(1), i.u16(2)));
        o.u16(svabalb_u16(i.u16(0), i.u8(1), i.u8(2)));
        o.u64(svabalt_u64(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svabalt_u32(i.u32(0), i.u16(1), i.u16(2)));
        o.u16(svabalt_u16(i.u16(0), i.u8(1), i.u8(2)));
        o.u64(svabdlb_u64(i.u32(0), i.u32(1)));
        o.u32(svabdlb_u32(i.u16(1), i.u16(2)));
        o.u16(svabdlb_u16(i.u8(1), i.u8(2)));
        o.u64(svabdlt_u64(i.u32(0), i.u32(1)));
        o.u32(svabdlt_u32(i.u16(1), i.u16(2)));
        o.u16(svabdlt_u16(i.u8(1), i.u8(2)));
        o.u64(svadalp_u64_m(i.p(1), i.u64(1), i.u32(2)));
        o.u32(svadalp_u32_m(i.p(0), i.u32(0), i.u16(1)));
        o.u16(svadalp_u16_m(i.p(0), i.u16(0), i.u8(1)));
        o.u64(svaddlb_u64(i.u32(0), i.u32(1)));
        o.u32(svaddlb_u32(i.u16(1), i.u16(2)));
        o.u16(svaddlb_u16(i.u8(1), i.u8(2)));
        o.u64(svaddlt_u64(i.u32(0), i.u32(1)));
        o.u32(svaddlt_u32(i.u16(1), i.u16(2)));
        o.u16(svaddlt_u16(i.u8(1), i.u8(2)));
        o.u64(svaddwb_u64(i.u64(1), i.u32(2)));
        o.u32(svaddwb_u32(i.u32(0), i.u16(1)));
        o.u16(svaddwb_u16(i.u16(0), i.u8(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group47(i: &In, o: &mut Out) {
    unsafe {
        o.u64(svaddwt_u64(i.u64(1), i.u32(2)));
        o.u32(svaddwt_u32(i.u32(0), i.u16(1)));
        o.u16(svaddwt_u16(i.u16(0), i.u8(1)));
        o.u8(svhadd_u8_m(i.p(0), i.u8(0), i.u8(1)));
        o.u64(svhadd_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svhadd_u32_m(i.p(1), i.u32(1), i.u32(2)));
        o.u16(svhadd_u16_m(i.p(1), i.u16(1), i.u16(2)));
        o.u8(svhsub_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svhsub_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svhsub_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svhsub_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.u8(svhsubr_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svhsubr_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svhsubr_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svhsubr_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.u8(svmaxp_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svmaxp_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svmaxp_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svmaxp_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.u8(svminp_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svminp_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svminp_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svminp_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.u64(svmlalb_lane_u64::<0>(i.u64(0), i.u32(1), i.u32(2)));
        o.u64(svmlalb_lane_u64::<3>(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlalb_lane_u32::<0>(i.u32(0), i.u16(1), i.u16(2)));
        o.u32(svmlalb_lane_u32::<7>(i.u32(1), i.u16(2), i.u16(3)));
        o.u64(svmlalb_u64(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlalb_u32(i.u32(0), i.u16(1), i.u16(2)));
        o.u16(svmlalb_u16(i.u16(0), i.u8(1), i.u8(2)));
        o.u64(svmlalt_lane_u64::<0>(i.u64(0), i.u32(1), i.u32(2)));
        o.u64(svmlalt_lane_u64::<3>(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlalt_lane_u32::<0>(i.u32(0), i.u16(1), i.u16(2)));
        o.u32(svmlalt_lane_u32::<7>(i.u32(1), i.u16(2), i.u16(3)));
        o.u64(svmlalt_u64(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlalt_u32(i.u32(0), i.u16(1), i.u16(2)));
        o.u16(svmlalt_u16(i.u16(0), i.u8(1), i.u8(2)));
        o.u64(svmlslb_lane_u64::<0>(i.u64(0), i.u32(1), i.u32(2)));
        o.u64(svmlslb_lane_u64::<3>(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlslb_lane_u32::<0>(i.u32(0), i.u16(1), i.u16(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group48(i: &In, o: &mut Out) {
    unsafe {
        o.u32(svmlslb_lane_u32::<7>(i.u32(1), i.u16(2), i.u16(3)));
        o.u64(svmlslb_u64(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlslb_u32(i.u32(0), i.u16(1), i.u16(2)));
        o.u16(svmlslb_u16(i.u16(0), i.u8(1), i.u8(2)));
        o.u64(svmlslt_lane_u64::<0>(i.u64(0), i.u32(1), i.u32(2)));
        o.u64(svmlslt_lane_u64::<3>(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlslt_lane_u32::<0>(i.u32(0), i.u16(1), i.u16(2)));
        o.u32(svmlslt_lane_u32::<7>(i.u32(1), i.u16(2), i.u16(3)));
        o.u64(svmlslt_u64(i.u64(1), i.u32(2), i.u32(3)));
        o.u32(svmlslt_u32(i.u32(0), i.u16(1), i.u16(2)));
        o.u16(svmlslt_u16(i.u16(0), i.u8(1), i.u8(2)));
        o.u64(svmullb_lane_u64::<0>(i.u32(0), i.u32(1)));
        o.u64(svmullb_lane_u64::<3>(i.u32(1), i.u32(2)));
        o.u32(svmullb_lane_u32::<0>(i.u16(0), i.u16(1)));
        o.u32(svmullb_lane_u32::<7>(i.u16(1), i.u16(2)));
        o.u64(svmullb_u64(i.u32(1), i.u32(2)));
        o.u32(svmullb_u32(i.u16(0), i.u16(1)));
        o.u16(svmullb_u16(i.u8(0), i.u8(1)));
        o.u64(svmullt_lane_u64::<0>(i.u32(0), i.u32(1)));
        o.u64(svmullt_lane_u64::<3>(i.u32(1), i.u32(2)));
        o.u32(svmullt_lane_u32::<0>(i.u16(0), i.u16(1)));
        o.u32(svmullt_lane_u32::<7>(i.u16(1), i.u16(2)));
        o.u64(svmullt_u64(i.u32(1), i.u32(2)));
        o.u32(svmullt_u32(i.u16(0), i.u16(1)));
        o.u16(svmullt_u16(i.u8(0), i.u8(1)));
        o.u8(svqadd_u8_m(i.p(1), i.u8(1), i.u8(2)));
        o.u64(svqadd_u64_m(i.p(1), i.u64(1), i.u64(2)));
        o.u32(svqadd_u32_m(i.p(0), i.u32(0), i.u32(1)));
        o.u16(svqadd_u16_m(i.p(0), i.u16(0), i.u16(1)));
        o.u8(svqrshl_u8_m(i.p(1), i.u8(1), i.s8(2)));
        o.u64(svqrshl_u64_m(i.p(1), i.u64(1), i.s64(2)));
        o.u32(svqrshl_u32_m(i.p(0), i.u32(0), i.s32(1)));
        o.u16(svqrshl_u16_m(i.p(0), i.u16(0), i.s16(1)));
        o.u32(svqrshrnb_n_u64::<1>(i.u64(0)));
        o.u32(svqrshrnb_n_u64::<32>(i.u64(1)));
        o.u16(svqrshrnb_n_u32::<1>(i.u32(0)));
        o.u16(svqrshrnb_n_u32::<16>(i.u32(1)));
        o.u8(svqrshrnb_n_u16::<1>(i.u16(0)));
        o.u8(svqrshrnb_n_u16::<8>(i.u16(1)));
        o.u32(svqrshrnt_n_u64::<1>(i.u32(0), i.u64(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group49(i: &In, o: &mut Out) {
    unsafe {
        o.u32(svqrshrnt_n_u64::<32>(i.u32(1), i.u64(2)));
        o.u16(svqrshrnt_n_u32::<1>(i.u16(0), i.u32(1)));
        o.u16(svqrshrnt_n_u32::<16>(i.u16(1), i.u32(2)));
        o.u8(svqrshrnt_n_u16::<1>(i.u8(0), i.u16(1)));
        o.u8(svqrshrnt_n_u16::<8>(i.u8(1), i.u16(2)));
        o.u8(svqshl_u8_m(i.p(1), i.u8(1), i.s8(2)));
        o.u64(svqshl_u64_m(i.p(1), i.u64(1), i.s64(2)));
        o.u32(svqshl_u32_m(i.p(0), i.u32(0), i.s32(1)));
        o.u16(svqshl_u16_m(i.p(0), i.u16(0), i.s16(1)));
        o.u32(svqshrnb_n_u64::<1>(i.u64(0)));
        o.u32(svqshrnb_n_u64::<32>(i.u64(1)));
        o.u16(svqshrnb_n_u32::<1>(i.u32(0)));
        o.u16(svqshrnb_n_u32::<16>(i.u32(1)));
        o.u8(svqshrnb_n_u16::<1>(i.u16(0)));
        o.u8(svqshrnb_n_u16::<8>(i.u16(1)));
        o.u32(svqshrnt_n_u64::<1>(i.u32(0), i.u64(1)));
        o.u32(svqshrnt_n_u64::<32>(i.u32(1), i.u64(2)));
        o.u16(svqshrnt_n_u32::<1>(i.u16(0), i.u32(1)));
        o.u16(svqshrnt_n_u32::<16>(i.u16(1), i.u32(2)));
        o.u8(svqshrnt_n_u16::<1>(i.u8(0), i.u16(1)));
        o.u8(svqshrnt_n_u16::<8>(i.u8(1), i.u16(2)));
        o.u8(svqsub_u8_m(i.p(0), i.u8(0), i.u8(1)));
        o.u64(svqsub_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svqsub_u32_m(i.p(1), i.u32(1), i.u32(2)));
        o.u16(svqsub_u16_m(i.p(1), i.u16(1), i.u16(2)));
        o.u8(svqsubr_u8_m(i.p(0), i.u8(0), i.u8(1)));
        o.u64(svqsubr_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svqsubr_u32_m(i.p(1), i.u32(1), i.u32(2)));
        o.u16(svqsubr_u16_m(i.p(1), i.u16(1), i.u16(2)));
        o.u32(svqxtnb_u64(i.u64(0)));
        o.u16(svqxtnb_u32(i.u32(1)));
        o.u8(svqxtnb_u16(i.u16(1)));
        o.u32(svqxtnt_u64(i.u32(0), i.u64(1)));
        o.u16(svqxtnt_u32(i.u16(1), i.u32(2)));
        o.u8(svqxtnt_u16(i.u8(1), i.u16(2)));
        o.u32(svrecpe_u32_m(i.u32(1), i.p(2), i.u32(2)));
        o.u8(svrhadd_u8_m(i.p(0), i.u8(0), i.u8(1)));
        o.u64(svrhadd_u64_m(i.p(0), i.u64(0), i.u64(1)));
        o.u32(svrhadd_u32_m(i.p(1), i.u32(1), i.u32(2)));
        o.u16(svrhadd_u16_m(i.p(1), i.u16(1), i.u16(2)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group50(i: &In, o: &mut Out) {
    unsafe {
        o.u8(svrshl_u8_m(i.p(0), i.u8(0), i.s8(1)));
        o.u64(svrshl_u64_m(i.p(0), i.u64(0), i.s64(1)));
        o.u32(svrshl_u32_m(i.p(1), i.u32(1), i.s32(2)));
        o.u16(svrshl_u16_m(i.p(1), i.u16(1), i.s16(2)));
        o.u8(svrshr_n_u8_m::<1>(i.p(0), i.u8(0)));
        o.u8(svrshr_n_u8_m::<8>(i.p(1), i.u8(1)));
        o.u64(svrshr_n_u64_m::<1>(i.p(0), i.u64(0)));
        o.u64(svrshr_n_u64_m::<64>(i.p(1), i.u64(1)));
        o.u32(svrshr_n_u32_m::<1>(i.p(0), i.u32(0)));
        o.u32(svrshr_n_u32_m::<32>(i.p(1), i.u32(1)));
        o.u16(svrshr_n_u16_m::<1>(i.p(0), i.u16(0)));
        o.u16(svrshr_n_u16_m::<16>(i.p(1), i.u16(1)));
        o.u32(svrsqrte_u32_m(i.u32(1), i.p(2), i.u32(2)));
        o.u8(svrsra_n_u8::<1>(i.u8(0), i.u8(1)));
        o.u8(svrsra_n_u8::<8>(i.u8(1), i.u8(2)));
        o.u64(svrsra_n_u64::<1>(i.u64(0), i.u64(1)));
        o.u64(svrsra_n_u64::<64>(i.u64(1), i.u64(2)));
        o.u32(svrsra_n_u32::<1>(i.u32(0), i.u32(1)));
        o.u32(svrsra_n_u32::<32>(i.u32(1), i.u32(2)));
        o.u16(svrsra_n_u16::<1>(i.u16(0), i.u16(1)));
        o.u16(svrsra_n_u16::<16>(i.u16(1), i.u16(2)));
        o.u64(svshllb_n_u64::<0>(i.u32(0)));
        o.u64(svshllb_n_u64::<31>(i.u32(1)));
        o.u32(svshllb_n_u32::<0>(i.u16(0)));
        o.u32(svshllb_n_u32::<15>(i.u16(1)));
        o.u16(svshllb_n_u16::<0>(i.u8(0)));
        o.u16(svshllb_n_u16::<7>(i.u8(1)));
        o.u64(svshllt_n_u64::<0>(i.u32(0)));
        o.u64(svshllt_n_u64::<31>(i.u32(1)));
        o.u32(svshllt_n_u32::<0>(i.u16(0)));
        o.u32(svshllt_n_u32::<15>(i.u16(1)));
        o.u16(svshllt_n_u16::<0>(i.u8(0)));
        o.u16(svshllt_n_u16::<7>(i.u8(1)));
        o.u8(svsqadd_u8_m(i.p(0), i.u8(0), i.s8(1)));
        o.u64(svsqadd_u64_m(i.p(0), i.u64(0), i.s64(1)));
        o.u32(svsqadd_u32_m(i.p(1), i.u32(1), i.s32(2)));
        o.u16(svsqadd_u16_m(i.p(1), i.u16(1), i.s16(2)));
        o.u8(svsra_n_u8::<1>(i.u8(0), i.u8(1)));
        o.u8(svsra_n_u8::<8>(i.u8(1), i.u8(2)));
        o.u64(svsra_n_u64::<1>(i.u64(0), i.u64(1)));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group51(i: &In, o: &mut Out) {
    unsafe {
        o.u64(svsra_n_u64::<64>(i.u64(1), i.u64(2)));
        o.u32(svsra_n_u32::<1>(i.u32(0), i.u32(1)));
        o.u32(svsra_n_u32::<32>(i.u32(1), i.u32(2)));
        o.u16(svsra_n_u16::<1>(i.u16(0), i.u16(1)));
        o.u16(svsra_n_u16::<16>(i.u16(1), i.u16(2)));
        o.u64(svsublb_u64(i.u32(1), i.u32(2)));
        o.u32(svsublb_u32(i.u16(0), i.u16(1)));
        o.u16(svsublb_u16(i.u8(0), i.u8(1)));
        o.u64(svsublt_u64(i.u32(1), i.u32(2)));
        o.u32(svsublt_u32(i.u16(0), i.u16(1)));
        o.u16(svsublt_u16(i.u8(0), i.u8(1)));
        o.u64(svsubwb_u64(i.u64(0), i.u32(1)));
        o.u32(svsubwb_u32(i.u32(1), i.u16(2)));
        o.u16(svsubwb_u16(i.u16(1), i.u8(2)));
        o.u64(svsubwt_u64(i.u64(0), i.u32(1)));
        o.u32(svsubwt_u32(i.u32(1), i.u16(2)));
        o.u16(svsubwt_u16(i.u16(1), i.u8(2)));
        o.u64(svunpkhi_u64(i.u32(0)));
        o.u32(svunpkhi_u32(i.u16(1)));
        o.u16(svunpkhi_u16(i.u8(1)));
        o.u64(svunpklo_u64(i.u32(0)));
        o.u32(svunpklo_u32(i.u16(1)));
        o.u16(svunpklo_u16(i.u8(1)));
        o.p(svwhilege_b8_s32(3i32, -1i32));
        o.p(svwhilege_b8_s64(i64::MIN, i64::MAX));
        o.p(svwhilege_b64_s32(3i32, -1i32));
        o.p(svwhilege_b64_s64(i64::MIN, i64::MAX));
        o.p(svwhilege_b32_s32(i32::MIN, i32::MAX));
        o.p(svwhilege_b32_s64(2i64, -1i64));
        o.p(svwhilege_b16_s32(i32::MIN, i32::MAX));
        o.p(svwhilege_b16_s64(2i64, -1i64));
        o.p(svwhilegt_b8_s32(i32::MIN, i32::MAX));
        o.p(svwhilegt_b8_s64(2i64, -1i64));
        o.p(svwhilegt_b64_s32(i32::MIN, i32::MAX));
        o.p(svwhilegt_b64_s64(2i64, -1i64));
        o.p(svwhilegt_b32_s32(3i32, -1i32));
        o.p(svwhilegt_b32_s64(i64::MIN, i64::MAX));
        o.p(svwhilegt_b16_s32(3i32, -1i32));
        o.p(svwhilegt_b16_s64(i64::MIN, i64::MAX));
        o.p(svwhilegt_b8_u32(u32::MAX, 0x8000_0000u32));
    }
}
#[target_feature(enable = "sve,sve2")]
unsafe fn group52(i: &In, o: &mut Out) {
    unsafe {
        o.p(svwhilegt_b8_u64(9u64, 1u64));
        o.p(svwhilegt_b64_u32(u32::MAX, 0x8000_0000u32));
        o.p(svwhilegt_b32_u32(5u32, 1u32));
        o.p(svwhilegt_b32_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilegt_b16_u32(5u32, 1u32));
        o.p(svwhilegt_b16_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilege_b8_u32(5u32, 1u32));
        o.p(svwhilege_b8_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilege_b64_u32(5u32, 1u32));
        o.p(svwhilege_b64_u64(u64::MAX, 1u64 << 63));
        o.p(svwhilege_b32_u32(u32::MAX, 0x8000_0000u32));
        o.p(svwhilege_b32_u64(9u64, 1u64));
        o.p(svwhilege_b16_u32(u32::MAX, 0x8000_0000u32));
        o.p(svwhilege_b16_u64(9u64, 1u64));
        o.s8(svxar_n_s8::<1>(i.s8(0), i.s8(1)));
        o.s8(svxar_n_s8::<8>(i.s8(1), i.s8(2)));
        o.s64(svxar_n_s64::<1>(i.s64(0), i.s64(1)));
        o.s64(svxar_n_s64::<64>(i.s64(1), i.s64(2)));
        o.s32(svxar_n_s32::<1>(i.s32(0), i.s32(1)));
        o.s32(svxar_n_s32::<32>(i.s32(1), i.s32(2)));
        o.s16(svxar_n_s16::<1>(i.s16(0), i.s16(1)));
        o.s16(svxar_n_s16::<16>(i.s16(1), i.s16(2)));
    }
}
#[target_feature(enable = "sve,sve2,sve2-aes")]
unsafe fn group53(i: &In, o: &mut Out) {
    unsafe {
        o.u8(svaesd_u8(i.u8(0), i.u8(1)));
        o.u8(svaese_u8(i.u8(1), i.u8(2)));
        o.u8(svaesimc_u8(i.u8(1)));
        o.u8(svaesmc_u8(i.u8(0)));
        o.u8(svpmullb_pair_u8(i.u8(0), i.u8(1)));
        o.u64(svpmullb_pair_u64(i.u64(0), i.u64(1)));
        o.u32(svpmullb_pair_u32(i.u32(1), i.u32(2)));
        o.u8(svpmullt_pair_u8(i.u8(0), i.u8(1)));
        o.u64(svpmullt_pair_u64(i.u64(0), i.u64(1)));
        o.u32(svpmullt_pair_u32(i.u32(1), i.u32(2)));
    }
}
#[target_feature(enable = "sve,sve2,sve2-bitperm")]
unsafe fn group54(i: &In, o: &mut Out) {
    unsafe {
        o.u8(svbdep_u8(i.u8(0), i.u8(1)));
        o.u64(svbdep_u64(i.u64(0), i.u64(1)));
        o.u32(svbdep_u32(i.u32(1), i.u32(2)));
        o.u16(svbdep_u16(i.u16(1), i.u16(2)));
        o.u8(svbext_u8(i.u8(0), i.u8(1)));
        o.u64(svbext_u64(i.u64(0), i.u64(1)));
        o.u32(svbext_u32(i.u32(1), i.u32(2)));
        o.u16(svbext_u16(i.u16(1), i.u16(2)));
        o.u8(svbgrp_u8(i.u8(0), i.u8(1)));
        o.u64(svbgrp_u64(i.u64(0), i.u64(1)));
        o.u32(svbgrp_u32(i.u32(1), i.u32(2)));
        o.u16(svbgrp_u16(i.u16(1), i.u16(2)));
    }
}
#[target_feature(enable = "sve,sve2,sve2-sha3")]
unsafe fn group55(i: &In, o: &mut Out) {
    unsafe {
        o.s64(svrax1_s64(i.s64(1), i.s64(2)));
    }
}
#[target_feature(enable = "sve,sve2,sve2-sm4")]
unsafe fn group56(i: &In, o: &mut Out) {
    unsafe {
        o.u32(svsm4e_u32(i.u32(1), i.u32(2)));
        o.u32(svsm4ekey_u32(i.u32(0), i.u32(1)));
    }
}
#[target_feature(enable = "sve")]
unsafe fn group57(i: &In, o: &mut Out) {
    unsafe {
        o.p(svdupq_n_b16(true, false, true, false, true, false, true, false));
        o.p(svdupq_n_b32(true, false, true, false));
        o.p(svdupq_n_b64(true, false));
        o.p(svdupq_n_b8(
            true, false, true, false, true, false, true, false, true, false, true, false, true,
            false, true, false,
        ));
        o.f32(svdupq_n_f32(1.5f32, -0.0f32, f32::NAN, f32::INFINITY));
        o.f64(svdupq_n_f64(f64::NAN, f64::NEG_INFINITY));
        o.s16(svdupq_n_s16(i16::MIN, i16::MAX, 300i16, -77i16, 0i16, -1i16, i16::MIN, i16::MAX));
        o.s32(svdupq_n_s32(i32::MIN, i32::MAX, 0i32, -70000i32));
        o.s64(svdupq_n_s64(2i64, -1i64));
        o.s8(svdupq_n_s8(
            0i8, 1i8, -1i8, -128i8, 127i8, 37i8, 0i8, 1i8, -1i8, -128i8, 127i8, 37i8, 0i8, 1i8,
            -1i8, -128i8,
        ));
        o.u16(svdupq_n_u16(u16::MAX, 0x8000u16, 1000u16, 13u16, 0u16, 1u16, u16::MAX, 0x8000u16));
        o.u32(svdupq_n_u32(u32::MAX, 0x8000_0000u32, 0u32, 65537u32));
        o.u64(svdupq_n_u64(9u64, 1u64));
        o.u8(svdupq_n_u8(
            0u8, 1u8, 255u8, 128u8, 7u8, 200u8, 0u8, 1u8, 255u8, 128u8, 7u8, 200u8, 0u8, 1u8,
            255u8, 128u8,
        ));
    }
}

fn bytes() -> u64 {
    #[target_feature(enable = "sve")]
    fn count() -> u64 {
        svcntb()
    }
    // SAFETY: only called once SVE is detected.
    unsafe { count() }
}

pub fn run() -> String {
    if !std::arch::is_aarch64_feature_detected!("sve2") {
        return "no sve2".to_owned();
    }
    let i = In::new();
    let mut o = Out { text: String::new(), memory: vec![0x5a; 4096] };
    // SAFETY: the machine has sve.
    unsafe { group0(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group1(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group2(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group3(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group4(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group5(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group6(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group7(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group8(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group9(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group10(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group11(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group12(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group13(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group14(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group15(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group16(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group17(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group18(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group19(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group20(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group21(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group22(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group23(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group24(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group25(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group26(&i, &mut o) }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("f32mm") {
        // SAFETY: the machine has sve,f32mm.
        unsafe { group27(&i, &mut o) }
    } else {
        o.text.push_str(" no f32mm ");
    }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("f64mm") && bytes() >= 32 {
        // SAFETY: the machine has sve,f64mm.
        unsafe { group28(&i, &mut o) }
    } else {
        o.text.push_str(" no f64mm+256-bit vectors ");
    }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("f64mm") && bytes() >= 32 {
        // SAFETY: the machine has sve,f64mm.
        unsafe { group29(&i, &mut o) }
    } else {
        o.text.push_str(" no f64mm+256-bit vectors ");
    }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("i8mm") {
        // SAFETY: the machine has sve,i8mm.
        unsafe { group30(&i, &mut o) }
    } else {
        o.text.push_str(" no i8mm ");
    }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group31(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group32(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group33(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group34(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group35(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group36(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group37(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group38(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group39(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group40(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group41(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group42(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group43(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group44(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group45(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group46(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group47(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group48(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group49(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group50(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group51(&i, &mut o) }
    o.text.push('\n');
    // SAFETY: the machine has sve,sve2.
    unsafe { group52(&i, &mut o) }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("sve2-aes") {
        // SAFETY: the machine has sve,sve2,sve2-aes.
        unsafe { group53(&i, &mut o) }
    } else {
        o.text.push_str(" no sve2-aes ");
    }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("sve2-bitperm") {
        // SAFETY: the machine has sve,sve2,sve2-bitperm.
        unsafe { group54(&i, &mut o) }
    } else {
        o.text.push_str(" no sve2-bitperm ");
    }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("sve2-sha3") {
        // SAFETY: the machine has sve,sve2,sve2-sha3.
        unsafe { group55(&i, &mut o) }
    } else {
        o.text.push_str(" no sve2-sha3 ");
    }
    o.text.push('\n');
    if std::arch::is_aarch64_feature_detected!("sve2-sm4") {
        // SAFETY: the machine has sve,sve2,sve2-sm4.
        unsafe { group56(&i, &mut o) }
    } else {
        o.text.push_str(" no sve2-sm4 ");
    }
    o.text.push('\n');
    // SAFETY: the machine has sve.
    unsafe { group57(&i, &mut o) }
    o.text.push('\n');
    o.text
}
