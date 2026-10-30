//! `ra-jit-rustc` under another path, for the first build of a workspace: cargo keeps
//! what it builds through one wrapper apart from what it builds through another, and in
//! that build the crates the daemon compiles are not rustc's work as in any other.

use std::process::ExitCode;

#[path = "ra-jit-rustc.rs"]
mod wrapper;

fn main() -> ExitCode {
    wrapper::main()
}
