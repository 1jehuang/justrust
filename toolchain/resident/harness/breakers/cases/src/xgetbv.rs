//! sha2 and libm ask which vector registers the system saves before they use AVX: `_xgetbv`
//! is `llvm.x86.xgetbv`, an instruction Cranelift has no name for.

#[cfg(target_arch = "x86_64")]
pub fn run() -> u64 {
    use std::arch::x86_64::{__cpuid, _xgetbv};
    // Bit 27 of `ecx` says the system uses `xsave`, without which `xgetbv` faults.
    if __cpuid(1).ecx & (1 << 27) == 0 {
        return 0;
    }
    unsafe { _xgetbv(0) }
}

#[cfg(not(target_arch = "x86_64"))]
pub fn run() -> u64 {
    0
}
