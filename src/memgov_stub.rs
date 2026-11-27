//! The memory governor's surface where it does not exist. Build hosts are
//! Linux and the governor (`memgov.rs`) is compiled only there. The shim
//! and the agent name these on every platform.

use crate::shim::Unit;

/// Set by the build-host agent for the builds it runs, nowhere else.
pub(crate) const ENV: &str = "JUSTRUST_MEMGOV";

/// Whether a value of [`ENV`] turns the governor on.
pub(crate) fn on(value: &str) -> bool {
    !matches!(value.trim(), "" | "0" | "off" | "false" | "no")
}

/// Nothing is held.
pub(crate) struct Ticket;

impl Ticket {
    pub(crate) fn stage(&self) -> &'static str {
        "wait-memory"
    }

    pub(crate) fn release(self) {}
}

/// Never waits.
pub(crate) fn admit(_: &Unit, _: &[std::ffi::OsString], _: &std::path::Path) -> Ticket {
    Ticket
}

pub(crate) fn learn(_: &Unit) {}
