//! Proc macros a caller runs outside the sandbox its proc-macro servers are started in
//! ([`proc_macro_api::wrap`]): their libraries are expanded by servers of their own, started
//! with [`proc_macro_api::wrap::UNSANDBOXED`] set, which the caller's wrapper leaves alone.

use std::sync::OnceLock;

use vfs::AbsPath;

/// Whether the proc macros of the crate named (as rustc names it) in the workspace at the
/// first path, whose library is at the second, run outside the sandbox. It is asked of each
/// library before a server is started for it.
pub type Unsandboxed = dyn Fn(&AbsPath, &str, &AbsPath) -> bool + Send + Sync;

static UNSANDBOXED: OnceLock<Box<Unsandboxed>> = OnceLock::new();

/// Has the proc macros `unsandboxed` names be expanded by servers started outside the
/// sandbox. The first call is the one kept.
pub fn unsandboxed_macros_with(unsandboxed: Box<Unsandboxed>) {
    _ = UNSANDBOXED.set(unsandboxed);
}

pub(crate) fn unsandboxed(workspace: &AbsPath, crate_name: &str, library: &AbsPath) -> bool {
    UNSANDBOXED.get().is_some_and(|it| it(workspace, crate_name, library))
}
