//! tests/ui/imports/extern-transmute.rs and tests/ui/abi/cross-crate/anon-extern-mod-cross-crate-2.rs:
//! a foreign function declared in an `extern` block of another crate of the workspace was
//! called under a name of ra-jit's own making, which nothing defines.

#[inline(never)]
fn magnitudes(small: i32, large: i64) -> (i32, i64) {
    unsafe { (foreign::abs(small), foreign::long_abs(large)) }
}

pub fn run() -> (i32, i64) {
    magnitudes(-3, -5_000_000_000)
}
