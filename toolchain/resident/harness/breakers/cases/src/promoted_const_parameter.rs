//! tests/ui/const-generics/promotion.rs: `&(3 + N)` is borrowed for `'static` though `N` is a
//! const parameter, since each instance knows its `N`. ra-jit kept the sum in the frame, and
//! the reference outlived it: Cranelift happened to leave the bytes alone, LLVM did not.

#[inline(never)]
fn sum<const N: usize>() -> &'static usize {
    &(3 + N)
}

#[inline(never)]
fn pair<const N: u8>() -> &'static (u8, u8) {
    &(N, N ^ 0xff)
}

#[inline(never)]
fn clobber(seed: u64) -> u64 {
    let filler = [seed; 64];
    filler.iter().fold(0, |acc, it| acc ^ it.rotate_left(7))
}

pub fn run() -> (usize, usize, (u8, u8), u64) {
    let (small, large, both) = (sum::<13>(), sum::<400>(), pair::<5>());
    let noise = clobber(std::hint::black_box(0x5555_5555_5555_5555));
    (*small, *large, *both, noise)
}
