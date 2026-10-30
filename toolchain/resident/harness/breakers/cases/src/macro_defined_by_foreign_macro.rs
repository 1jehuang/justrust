//! bun (src/runtime/ipc.rs): `bun_core::define_scoped_log!(log, IPC, visible)` at the top of a
//! file defines `log!` with `macro_rules!`, and the functions of an inline module of the file call
//! it. rust-analyzer expanded a call by a path (`foreign::...!`) only once the file's modules were
//! collected, so the `macro_rules!` it made was not in their textual scope: the call was a missing
//! expression, and every function of the module calling it was left to rustc (`IncompleteExpr`).

foreign::define_scaled!(tripled, 3);

pub fn run() -> (u64, u64) {
    (tripled!(5), inner::scaled(7))
}

mod inner {
    pub fn scaled(n: u64) -> u64 {
        tripled!(n + 1)
    }
}
