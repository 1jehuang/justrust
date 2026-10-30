//! Proc macros compiled to WebAssembly, which no proc-macro server loads: a caller that runs
//! them (ra-jit, `crates/ra-jit-macro-wasm`) loads them itself, in its own process.

use std::sync::OnceLock;

use hir_expand::proc_macro::ProcMacroLoadResult;
use ide_db::base_db::ProcMacroLoadingError;
use vfs::AbsPath;

/// Loads the macros of the module at a path.
pub type LoadWasm = dyn Fn(&AbsPath) -> ProcMacroLoadResult + Send + Sync;

static LOAD: OnceLock<Box<LoadWasm>> = OnceLock::new();

/// Has every module of proc macros a load names loaded by `load`. The first call is kept.
pub fn wasm_macros_with(load: Box<LoadWasm>) {
    _ = LOAD.set(load);
}

/// Whether the proc macro at `path` is a WebAssembly module rather than a library, which it
/// can be under a library's name, where cargo looks for the library.
pub fn is_wasm(path: &AbsPath) -> bool {
    use std::io::Read as _;
    let mut magic = [0; 4];
    path.extension() == Some("wasm")
        || std::fs::File::open(path).and_then(|mut it| it.read_exact(&mut magic)).is_ok()
            && magic == *b"\0asm"
}

pub(crate) fn load(path: &AbsPath) -> ProcMacroLoadResult {
    match LOAD.get() {
        Some(load) => load(path),
        None => Err(ProcMacroLoadingError::ProcMacroSrvError(
            format!("nothing here runs the proc macros compiled to WebAssembly at {path}").into(),
        )),
    }
}
