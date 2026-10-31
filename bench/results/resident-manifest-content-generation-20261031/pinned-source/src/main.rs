//! Entry point. The implementation lives in `lib.rs` and is Linux only.

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    justrust::main()
}

/// Not Linux: fail open. `justrust check|test|build|clippy|run|cargo ...` and
/// the `cargo` proxy name run the real cargo unchanged, without recording.
#[cfg(not(target_os = "linux"))]
fn main() {
    let mut args = std::env::args_os();
    let argv0 = args.next().unwrap_or_default();
    let rest: Vec<std::ffi::OsString> = args.collect();
    let invoked_as_cargo = std::path::Path::new(&argv0)
        .file_stem()
        .is_some_and(|n| n == "cargo");
    let cargo_args: Vec<std::ffi::OsString> = if invoked_as_cargo {
        rest
    } else {
        match rest.first().and_then(|a| a.to_str()) {
            Some("check" | "test" | "build" | "clippy" | "run") => rest,
            Some("cargo") => rest[1..].to_vec(),
            Some("-V" | "--version") => {
                println!("justrust {} (passthrough mode)", env!("CARGO_PKG_VERSION"));
                return;
            }
            _ => {
                eprintln!(
                    "justrust {}: only Linux is supported for now. On this platform \
                     `justrust check|test|build|clippy|run` run plain cargo without \
                     recording, and the other commands are unavailable.",
                    env!("CARGO_PKG_VERSION")
                );
                std::process::exit(2);
            }
        }
    };
    if invoked_as_cargo {
        eprintln!("justrust: cargo proxy is Linux only, running cargo unchanged");
    } else {
        eprintln!("justrust: not on Linux, running plain cargo without recording");
    }
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    match std::process::Command::new(cargo).args(&cargo_args).status() {
        Ok(s) => std::process::exit(s.code().unwrap_or(1)),
        Err(e) => {
            eprintln!("justrust: failed to run cargo: {e}");
            std::process::exit(127);
        }
    }
}
