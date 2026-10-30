//! What a caller of this crate puts around each proc-macro expander it loads (ra-jit keeps
//! expansions across processes with it), and whether an expansion asked anything of the
//! database while it ran, which an expansion that is kept must not have.

use std::{cell::Cell, sync::Arc, sync::OnceLock};

use hir_expand::proc_macro::{ProcMacroExpander, ProcMacroKind};
use vfs::AbsPath;

/// Takes the expander of the macro `name` of the dylib at the path, of its kind, and gives
/// back the one to expand with.
pub type Wrap =
    fn(Arc<dyn ProcMacroExpander>, &AbsPath, &str, ProcMacroKind) -> Arc<dyn ProcMacroExpander>;

static WRAP: OnceLock<Wrap> = OnceLock::new();

thread_local! {
    static ASKED: Cell<u64> = const { Cell::new(0) };
}

/// Puts `wrap` around every expander loaded from here on. The first call is the one kept.
pub fn wrap_expanders(wrap: Wrap) {
    _ = WRAP.set(wrap);
}

/// How many questions the expansions on this thread have asked of the database so far: an
/// expansion asked none when this is the same after it as before it.
pub fn asked() -> u64 {
    ASKED.with(Cell::get)
}

pub(crate) fn ask() {
    ASKED.with(|it| it.set(it.get() + 1));
}

pub(crate) fn wrapped(
    expander: Arc<dyn ProcMacroExpander>,
    dylib: &AbsPath,
    name: &str,
    kind: ProcMacroKind,
) -> Arc<dyn ProcMacroExpander> {
    match WRAP.get() {
        Some(wrap) => wrap(expander, dylib, name, kind),
        None => expander,
    }
}
