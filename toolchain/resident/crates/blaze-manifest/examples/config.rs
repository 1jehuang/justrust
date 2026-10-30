//! `cargo config get` as blaze-manifest reads the configuration, for `harness/resolve.py` to
//! compare with `cargo -Zunstable-options config get`: takes what follows `cargo`, as in
//! `cargo run -p blaze-manifest --example config -- config get --format json build.target`.

#![allow(clippy::print_stderr, clippy::print_stdout, reason = "a command line program")]

use std::process::ExitCode;

use blaze_manifest::config::get::main as config_main;

fn main() -> ExitCode {
    let cwd = std::env::current_dir().unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (code, output) = config_main(&cwd, std::env::vars(), &args);
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    ExitCode::from(code)
}
