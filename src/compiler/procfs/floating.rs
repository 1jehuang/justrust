//! Admission of the floating-point environment assumed by Rust/LLVM.
//! Read control state only. Sticky exception status is not a semantic input.
use std::io;

#[cfg(target_arch = "x86_64")]
pub(super) fn current() -> io::Result<[u8; 6]> {
    let mut mxcsr = 0u32;
    let mut x87 = 0u16;
    // Both stores target initialized, naturally aligned local values. Do not
    // mark these reads pure: control state can change between observations.
    unsafe {
        std::arch::asm!(
            "stmxcsr [{}]",
            "fnstcw [{}]",
            in(reg) &mut mxcsr,
            in(reg) &mut x87,
            options(nostack, preserves_flags),
        );
    }
    admit(mxcsr, x87)
}

#[cfg(target_arch = "x86_64")]
fn admit(mxcsr: u32, x87: u16) -> io::Result<[u8; 6]> {
    let controls = mxcsr & !0x3f;
    // Round nearest/even, masked exceptions, preserved subnormals, and the
    // native x87 extended-precision default. Nondefault environments cannot
    // mint runtime evidence, not merely a differently keyed compiled result.
    if controls != 0x1f80 || x87 != 0x037f {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "unsupported compiler floating-point controls: MXCSR={controls:#06x}, x87={x87:#06x}"
            ),
        ));
    }
    let mut bytes = [0; 6];
    bytes[..4].copy_from_slice(&controls.to_le_bytes());
    bytes[4..].copy_from_slice(&x87.to_le_bytes());
    Ok(bytes)
}

#[cfg(not(target_arch = "x86_64"))]
pub(super) fn current() -> io::Result<[u8; 6]> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "compiler floating-point runtime evidence requires x86-64",
    ))
}

#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    use super::*;

    struct Restore {
        mxcsr: u32,
        x87: u16,
    }
    impl Restore {
        fn new() -> Self {
            let mut saved = Self { mxcsr: 0, x87: 0 };
            unsafe {
                std::arch::asm!(
                    "stmxcsr [{}]", "fnstcw [{}]",
                    in(reg) &mut saved.mxcsr, in(reg) &mut saved.x87,
                    options(nostack, preserves_flags),
                );
            }
            saved
        }
        fn set(&self, mxcsr: u32, x87: u16) {
            unsafe {
                std::arch::asm!(
                    "ldmxcsr [{}]", "fldcw [{}]",
                    in(reg) &mxcsr, in(reg) &x87,
                    options(nostack, preserves_flags),
                );
            }
        }
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            self.set(self.mxcsr, self.x87);
        }
    }

    #[test]
    fn floating_control_admission_rejects_unknown_and_each_nondefault_control_bit() {
        let baseline = admit(0x1f80, 0x037f).unwrap();
        for status in 0..64 {
            assert_eq!(admit(0x1f80 | status, 0x037f).unwrap(), baseline);
        }
        for bit in 6..32 {
            assert!(admit(0x1f80 ^ (1 << bit), 0x037f).is_err());
        }
        for bit in 0..16 {
            assert!(admit(0x1f80, 0x037f ^ (1 << bit)).is_err());
        }
    }

    #[test]
    fn floating_controls_prevent_stale_runtime_evidence() {
        const CHILD: &str = "JUSTRUST_FP_EVIDENCE_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "compiler::procfs::floating::tests::floating_controls_prevent_stale_runtime_evidence",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            return;
        }
        let saved = Restore::new();
        let baseline = crate::compiler::procfs::CompilerImage::current().unwrap();
        saved.set((saved.mxcsr & !0x3f) ^ 0x2000, saved.x87);
        // No Rust floating operations execute under the unsupported mode.
        let unsupported = crate::compiler::procfs::CompilerImage::current();
        let stale = baseline.validate_current();
        saved.set(saved.mxcsr, saved.x87);
        assert_eq!(unsupported.unwrap_err().kind(), io::ErrorKind::Unsupported);
        assert!(stale.is_err());
        assert_eq!(
            crate::compiler::procfs::CompilerImage::current().unwrap(),
            baseline
        );
        saved.set(saved.mxcsr | 0x3f, saved.x87);
        let status_only = crate::compiler::procfs::CompilerImage::current();
        saved.set(saved.mxcsr, saved.x87);
        assert_eq!(status_only.unwrap(), baseline);
    }

    #[test]
    fn floating_controls_reject_unsupported_environment_but_ignore_sticky_status() {
        let saved = Restore::new();
        let baseline = current().unwrap();
        for (mxcsr, x87) in [
            ((saved.mxcsr & !0x3f) ^ 0x2000, saved.x87), // directed SSE rounding
            ((saved.mxcsr & !0x3f) ^ 0x8000, saved.x87), // flush-to-zero
            ((saved.mxcsr & !0x3f) ^ 0x0080, saved.x87), // unmasked invalid trap
            (saved.mxcsr & !0x3f, saved.x87 ^ 0x0400),   // directed x87 rounding
            (saved.mxcsr & !0x3f, saved.x87 ^ 0x0100),   // nondefault x87 precision
            (saved.mxcsr & !0x3f, saved.x87 ^ 0x0001),   // unmasked x87 invalid trap
        ] {
            saved.set(mxcsr, x87);
            let rejected = current().is_err();
            saved.set(saved.mxcsr, saved.x87);
            assert!(rejected, "accepted MXCSR={mxcsr:#x}, x87={x87:#x}");
        }
        saved.set(saved.mxcsr | 0x3f, saved.x87);
        let sticky = current();
        saved.set(saved.mxcsr, saved.x87);
        assert_eq!(sticky.unwrap(), baseline);
    }
}
