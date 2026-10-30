//! The `profile` of every unit of a `cargo <command> --unit-graph` as blaze-manifest computes
//! it, for `harness/resolve.py` to compare with cargo's: `cargo +1.99.0 build --unit-graph
//! -Zunstable-options | profiles --host aarch64-apple-darwin build`. Takes cargo's arguments
//! after the host triple and prints a JSON array, one profile per unit of the graph.

#![allow(clippy::print_stderr, clippy::print_stdout, reason = "a command line program")]

mod walk;

use std::io::Read as _;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (Some("--host"), Some(host)) = (args.first().map(String::as_str), args.get(1)) else {
        eprintln!("usage: profiles --host TRIPLE COMMAND [ARGS...] < UNIT-GRAPH");
        return ExitCode::from(1);
    };
    let mut graph = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut graph) {
        eprintln!("error: reading the unit graph: {err}");
        return ExitCode::from(101);
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let output = walk::main(&cwd, std::env::vars(), host, &args[2..], &graph);
    print!("{}", output.stdout);
    eprint!("{}", output.stderr);
    ExitCode::from(output.code)
}
