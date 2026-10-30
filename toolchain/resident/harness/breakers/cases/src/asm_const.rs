//! lzma-rust2's range decoder (through blaze's own downloads) gives `asm!` an unsuffixed
//! literal, `const 0x0100_0000`, and a constant of the crate, `const SHIFT_BITS`, in a method
//! of a generic type: ra-jit refused the `asm!` as "a constant that is not a literal", and
//! the whole build of blaze with it.

#[cfg(target_arch = "x86_64")]
const SHIFT_BITS: u32 = 8;

#[cfg(target_arch = "x86_64")]
struct Decoder<R> {
    range: u32,
    source: R,
}

#[cfg(target_arch = "x86_64")]
impl<R: Copy + Into<u64>> Decoder<R> {
    #[inline(always)]
    fn step(&mut self) -> (u32, u64) {
        let mut range = self.range;
        let shifted: u64;
        unsafe {
            std::arch::asm!(
                "cmp {range:e}, {top_value}",
                "jae 2f",
                "shl {range:e}, {shift_bits}",
                "2:",
                "mov {out}, {top_value} + {shift_bits}",
                range = inout(reg) range,
                out = out(reg) shifted,
                top_value = const 0x0100_0000,
                shift_bits = const SHIFT_BITS,
                options(nomem, nostack),
            );
        }
        self.range = range;
        (range, shifted + self.source.into())
    }
}

#[cfg(target_arch = "x86_64")]
pub fn run() -> Vec<(u32, u64)> {
    let mut decoder = Decoder { range: 0x1234, source: 7u8 };
    let mut big = Decoder { range: 0x0200_0000, source: 9u32 };
    vec![decoder.step(), decoder.step(), decoder.step(), big.step()]
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> Vec<(u32, u64)> {
    Vec::new()
}
