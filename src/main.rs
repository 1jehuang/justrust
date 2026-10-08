//! justrust: an attempt at a faster all-in-one Rust compiling solution for coding agents.
//!
//! One binary, three entry points chosen by the name it is run as:
//! - `cargo` (via `justrust install`): records the build, then behaves like cargo.
//! - `rustc` (the shim cargo uses during a recorded run): times each unit.
//! - `justrust`: the CLI below. `justrust check|test|build|clippy|run` is the
//!   interface agents should use. It takes the same arguments as cargo.

mod agent_output;
mod depcache;
mod findings;
mod history;
mod install;
mod paths;
mod procfs;
mod record;
mod runs;
mod shim;
mod slots;
mod summary;

use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

const AFTER_HELP: &str = "\
Agents: use `justrust check`, `justrust test`, `justrust build`, `justrust clippy`,
and `justrust run` instead of the matching cargo commands. They take exactly the
same arguments as cargo, record a timing profile, and print compact output:
errors and failing tests in full, the first few warnings, no progress noise,
then a one-line verdict. The full output is always saved: `justrust log`.

Examples:
  justrust check -p my-crate
  justrust test -p my-crate --lib some_module::
  justrust test -p my-crate -- --nocapture
  justrust log --grep warning      full output of the last run
  justrust show                    where the time went in the last run";

#[derive(Parser)]
#[command(name = "justrust", version, about, after_help = AFTER_HELP)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

/// Arguments passed through to cargo unchanged.
#[derive(clap::Args)]
struct Passthrough {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<OsString>,
}

#[derive(Subcommand)]
enum Command {
    /// `cargo check` with compact agent output and a timing profile.
    #[command(disable_help_flag = true)]
    Check(Passthrough),
    /// `cargo test` with compact agent output and a timing profile.
    #[command(disable_help_flag = true)]
    Test(Passthrough),
    /// `cargo build` with compact agent output and a timing profile.
    #[command(disable_help_flag = true)]
    Build(Passthrough),
    /// `cargo clippy` with compact agent output and a timing profile.
    #[command(disable_help_flag = true)]
    Clippy(Passthrough),
    /// `cargo run` with compact agent output and a timing profile.
    #[command(disable_help_flag = true)]
    Run(Passthrough),
    /// Run any cargo command, recording it with normal cargo output.
    #[command(disable_help_flag = true)]
    Cargo(Passthrough),
    /// Print the full saved output of a run (default: the latest).
    Log {
        /// Run id or prefix, or "last".
        id: Option<String>,
        /// Only lines containing this text.
        #[arg(long)]
        grep: Option<String>,
        /// Only the last N lines.
        #[arg(long)]
        tail: Option<usize>,
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
    /// List this workspace's per-agent build slots, or remove the free ones.
    Slots {
        /// Remove every slot that is not in use.
        #[arg(long)]
        clean: bool,
        /// Run the slot GC now (idle age and exclusive-size budget).
        #[arg(long)]
        gc: bool,
        /// Re-measure slot sizes instead of using the last GC's numbers.
        #[arg(long)]
        du: bool,
        /// Internal: background GC of the slots under this dir.
        #[arg(long, hide = true)]
        gc_root: Option<PathBuf>,
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

fn agent(sub: &str, args: Vec<OsString>) -> ! {
    let mut full = vec![OsString::from(sub)];
    full.extend(args);
    let max_warnings = std::env::var("JUSTRUST_MAX_WARNINGS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(agent_output::DEFAULT_MAX_WARNINGS);
    record::run(
        full,
        record::Options {
            agent: true,
            max_warnings,
        },
    )
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
        Command::Check(p) => agent("check", p.args),
        Command::Test(p) => agent("test", p.args),
        Command::Build(p) => agent("build", p.args),
        Command::Clippy(p) => agent("clippy", p.args),
        Command::Run(p) => agent("run", p.args),
        Command::Cargo(p) => record::main(p.args),
        Command::Log { id, grep, tail } => runs::log(id.as_deref(), grep.as_deref(), tail)?,
        Command::Runs { limit, here, json } => runs::list(limit, here, json)?,
        Command::Show { id, json } => runs::show(id.as_deref(), json)?,
        Command::Slots {
            clean,
            gc,
            du,
            gc_root,
        } => slots::command(clean, gc, du, gc_root)?,
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
