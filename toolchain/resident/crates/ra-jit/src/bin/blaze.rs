//! `blaze build|run|test|check|stop ...`: `cargo blaze ...` for a machine with no cargo of its
//! own, which becomes `ra-jit cargo ...`, the `ra-jit` being the one installed next to it.

use std::process::ExitCode;

#[path = "cargo-blaze.rs"]
mod cargo_blaze;

fn main() -> ExitCode {
    cargo_blaze::launch("blaze", None)
}
