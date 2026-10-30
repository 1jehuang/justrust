//! `cargo blaze ...`: cargo runs this as `cargo-blaze blaze ...`, and it becomes
//! `ra-jit cargo ...`, the `ra-jit` being the one installed next to it.

use std::process::{Command, ExitCode};

#[allow(dead_code)]
fn main() -> ExitCode {
    launch("cargo-blaze", Some("blaze"))
}

/// Runs the `ra-jit` next to this program, `this`, as `ra-jit cargo <args>`, without
/// `subcommand`, the name cargo passes first when it runs a subcommand.
pub(crate) fn launch(this: &str, subcommand: Option<&str>) -> ExitCode {
    let exe = match std::env::current_exe() {
        Ok(this) => this.with_file_name(format!("ra-jit{}", std::env::consts::EXE_SUFFIX)),
        Err(error) => {
            eprintln!("error: {this} does not know where it is installed: {error}");
            return ExitCode::FAILURE;
        }
    };
    let mut args = std::env::args_os().skip(1).peekable();
    if let Some(subcommand) = subcommand {
        while args.next_if(|it| it == subcommand).is_some() {}
    }
    match Command::new(&exe).arg("cargo").args(args).status() {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1) as u8),
        Err(error) => {
            eprintln!(
                "error: {this} could not run {}, which is installed with it ({error}); \
                 installing blaze again puts it back",
                exe.display()
            );
            ExitCode::FAILURE
        }
    }
}
