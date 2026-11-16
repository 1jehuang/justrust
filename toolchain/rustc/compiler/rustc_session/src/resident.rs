//! Native resident-driver process identity, not a session option or environment flag.
//!
//! The driver opts in once, after validating its protocol descriptors and before
//! entering any compiler session. The flag cannot be disabled: process-owned
//! resources may already have survived an earlier session.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

struct ProcessFlag(AtomicBool);

impl ProcessFlag {
    const fn new() -> Self {
        Self(AtomicBool::new(false))
    }

    fn enable(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_enabled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

static PROCESS: ProcessFlag = ProcessFlag::new();
static FRONTEND_REQUESTED: OnceLock<bool> = OnceLock::new();

/// Whether the explicit native resident driver may retain owned frontend snapshots.
///
/// This is process configuration, never a tracked compiler option or cache-format
/// change. One-shot rustc cannot enable it by inheriting the environment variable.
/// The driver freezes its environment before the first session.
pub fn frontend_enabled() -> bool {
    is_process()
        && *FRONTEND_REQUESTED.get_or_init(|| {
            std::env::var_os("JUSTRUST_RUSTC_RESIDENT_FRONTEND").is_some_and(|value| value == "1")
        })
}

/// Mark this process as a native resident worker before its first compiler session.
///
/// Only the native driver should call this. This is intentionally independent of
/// tracked options and environment variables, including resident value retention.
pub fn enable_process() {
    PROCESS.enable();
}

/// Whether the driver permanently opted this process into native resident mode.
pub fn is_process() -> bool {
    PROCESS.is_enabled()
}

#[cfg(test)]
mod tests {
    use super::ProcessFlag;

    #[test]
    fn fresh_process_flag_is_disabled() {
        // A local instance avoids mutating the global process identity and
        // interfering with unrelated unit tests running in this same process.
        let flag = ProcessFlag::new();
        assert!(!flag.is_enabled());
    }

    #[test]
    fn enable_is_permanent_and_idempotent() {
        let flag = ProcessFlag::new();
        flag.enable();
        assert!(flag.is_enabled());
        flag.enable();
        assert!(flag.is_enabled());
    }
}
