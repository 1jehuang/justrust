//! `cargo ra-jit ...`, the name `cargo blaze` had before: the same command, which says once
//! per run on a terminal that it was renamed.

use std::{io::IsTerminal as _, process::ExitCode};

#[path = "cargo-blaze.rs"]
mod blaze;

fn main() -> ExitCode {
    // Scripts and the harnesses read what it prints, and keep working unchanged.
    if std::io::stderr().is_terminal() {
        eprintln!("note: `cargo ra-jit` is now `cargo blaze`; the old name keeps working for now");
    }
    blaze::launch("cargo-ra-jit", Some("ra-jit"))
}
