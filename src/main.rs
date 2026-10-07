//! justrust: an attempt at a faster all-in-one Rust compiling solution for coding agents.
//!
//! One binary, three entry points chosen by the name it is run as:
//! - `cargo` (via `justrust install`): records the build, then behaves like cargo.
//! - `rustc` (the shim cargo uses during a recorded run): times each unit.
//! - `justrust`: the CLI below.

mod history;
mod install;
mod paths;
mod procfs;
mod record;
mod runs;
mod shim;
mod summary;

use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "justrust", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run cargo and record a full timing and resource profile of the build.
    #[command(disable_help_flag = true, allow_hyphen_values = true)]
    Cargo {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// List recorded runs.
    Runs {
        /// Number of runs to show.
        #[arg(short = 'n', long, default_value_t = 20)]
        limit: usize,
        /// Only runs started in or below the current directory.
        #[arg(long)]
        here: bool,
        #[arg(long)]
        json: bool,
    },
    /// Show where the time went in a recorded run (default: the latest).
    Show {
        /// Run id or prefix, or "last".
        id: Option<String>,
        #[arg(long)]
        json: bool,
    },
    /// Install a `cargo` proxy so every build is recorded automatically.
    Install {
        /// Directory for the proxy. Must come before the real cargo on PATH.
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Remove the `cargo` proxy.
    Uninstall {
        #[arg(long)]
        dir: Option<PathBuf>,
    },
    /// Analyze cargo invocations recorded in Jcode session history.
    History {
        /// Session directory. Defaults to ~/.jcode/sessions.
        #[arg(long)]
        sessions: Option<PathBuf>,
        /// Only include sessions whose working directory contains this string.
        #[arg(long)]
        repo: Option<String>,
        /// Ignore calls longer than this many seconds (hung processes).
        #[arg(long, default_value_t = 1800.0)]
        max_wall: f64,
        /// Write every extracted call as JSON lines to this file.
        #[arg(long)]
        dump: Option<PathBuf>,
        /// Number of command patterns to list.
        #[arg(long, default_value_t = 15)]
        top: usize,
    },
}

fn main() -> anyhow::Result<()> {
    let argv0 = std::env::args_os().next().unwrap_or_default();
    let name = std::path::Path::new(&argv0)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if shim::invoked_as_shim(&argv0) {
        shim::main();
    }
    if name == "cargo" {
        record::main(std::env::args_os().skip(1).collect());
    }

    let cli = Cli::parse();
    match cli.command {
        Command::Cargo { args } => record::main(args),
        Command::Runs { limit, here, json } => runs::list(limit, here, json)?,
        Command::Show { id, json } => runs::show(id.as_deref(), json)?,
        Command::Install { dir } => install::install(dir)?,
        Command::Uninstall { dir } => install::uninstall(dir)?,
        Command::History {
            sessions,
            repo,
            max_wall,
            dump,
            top,
        } => {
            let dir = match sessions {
                Some(d) => d,
                None => dirs::home_dir()
                    .ok_or_else(|| anyhow::anyhow!("no home directory"))?
                    .join(".jcode/sessions"),
            };
            let calls = history::extract(&dir)?;
            if let Some(path) = dump {
                history::dump(&calls, &path)?;
            }
            let report = history::Report::build(&calls, repo.as_deref(), max_wall, top);
            print!("{report}");
        }
    }
    Ok(())
}
