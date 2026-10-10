//! justrust: fast Rust compile for coding agents.
//!
//! One binary, three entry points chosen by the name it is run as:
//! - `cargo` (via `justrust install`): records the build, then behaves like cargo.
//! - `rustc` (the shim cargo uses during a recorded run): times each unit.
//! - `justrust`: the CLI below. `justrust check|test|build|clippy|run` is the
//!   interface agents should use. It takes the same arguments as cargo.

mod agent_output;
mod buildscript;
mod depcache;
mod depcache_gc;
mod findings;
mod history;
mod install;
mod live;
mod paths;
mod procfs;
mod record;
mod remote;
mod remote_agent;
mod remote_backend;
mod remote_build;
mod remote_daemon;
mod remote_proto;
mod remote_sync;
mod remote_watch;
mod runs;
mod sched;
mod shim;
mod slots;
mod split;
mod split_apply;
mod status;
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
    /// What is compiling right now, how far along, and the estimated time left.
    Status {
        #[arg(long)]
        json: bool,
        /// One line of Waybar custom-module JSON.
        #[arg(long)]
        waybar: bool,
        /// Print again every N seconds (for Waybar's continuous `exec`).
        #[arg(long, value_name = "SECS")]
        watch: Option<f64>,
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
    /// Show the shared dependency cache: size, entries, hit rate.
    Cache {
        /// Evict least recently used entries down to the size cap
        /// (`JUSTRUST_DEPCACHE_MAX`, default 30G).
        #[arg(long)]
        prune: bool,
        /// Remove every entry not used in the last 10 minutes.
        #[arg(long)]
        clear: bool,
        /// Background collection after a run (internal).
        #[arg(long, hide = true)]
        auto: bool,
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
    /// Which code to move into its own crate to stop rebuild cascades,
    /// from recorded runs in this workspace and its module graph.
    Split {
        /// Only runs from the last N days.
        #[arg(long, default_value_t = 30.0)]
        days: f64,
        /// Number of recommendations.
        #[arg(long, default_value_t = 8)]
        top: usize,
        /// Replay these recorded run ids instead of this workspace's history
        /// (for checking estimates against runs made in a scratch worktree).
        #[arg(long, hide = true, num_args = 1..)]
        runs: Vec<String>,
        /// Do the split: move <crate>::<module> into a new crate in this
        /// working tree. Requires a clean git tree (the pre-split commit),
        /// verifies with cargo, and commits the result; on failure restores
        /// the tree.
        #[arg(long, value_name = "CRATE::MODULE")]
        apply: Option<String>,
        /// Name of the new crate (default: <crate>-<module>).
        #[arg(long, requires = "apply")]
        name: Option<String>,
        /// With --apply: print the plan and change nothing.
        #[arg(long, requires = "apply")]
        dry_run: bool,
    },
    /// The remote compile machine: create, start, stop, status, ssh.
    Remote {
        #[command(subcommand)]
        cmd: RemoteCmd,
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

#[derive(Subcommand)]
enum RemoteCmd {
    /// Create the machine, or start it if it is stopped.
    Up {
        /// AWS region (default us-west-2).
        #[arg(long)]
        region: Option<String>,
        /// EC2 instance type (default c7i.8xlarge, 32 vCPUs).
        #[arg(long = "type")]
        instance_type: Option<String>,
        /// On-demand instead of spot.
        #[arg(long)]
        on_demand: bool,
        /// Minutes without load or a login before it stops itself (default 30).
        #[arg(long)]
        idle_minutes: Option<u32>,
    },
    /// Stop the machine (the disk and its caches are kept).
    Down,
    /// Terminate the machine and delete its disk.
    Destroy {
        #[arg(long)]
        yes: bool,
    },
    /// State, cost, round trip, load, and toolchain of the machine.
    Status {
        #[arg(long)]
        json: bool,
        /// One line of Waybar custom-module JSON.
        #[arg(long)]
        waybar: bool,
        /// Print again every N seconds.
        #[arg(long, value_name = "SECS")]
        watch: Option<f64>,
    },
    /// Open a shell on the machine, or run a command there.
    Ssh {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        cmd: Vec<String>,
    },
    /// `justrust check` on the machine (syncs the source first).
    #[command(disable_help_flag = true)]
    Check(RemoteArgs),
    /// `justrust test` on the machine.
    #[command(disable_help_flag = true)]
    Test(RemoteArgs),
    /// `justrust build` on the machine.
    #[command(disable_help_flag = true)]
    Build(RemoteArgs),
    /// `justrust clippy` on the machine.
    #[command(disable_help_flag = true)]
    Clippy(RemoteArgs),
    /// Choose where remote builds run: aws (the machine `remote up`
    /// creates), ssh HOST (a machine you already have), hosted (Jcode
    /// subscription, not available yet), or off.
    Use {
        kind: String,
        host: Option<String>,
        #[arg(long)]
        port: Option<u16>,
        /// ssh private key for the host.
        #[arg(long)]
        identity: Option<String>,
    },
}

#[derive(clap::Args)]
struct RemoteArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
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
    // `justrust __build-script <real> ...`: the build-script run wrapper.
    let mut rest = std::env::args_os().skip(1);
    if rest.next().as_deref() == Some(std::ffi::OsStr::new("__build-script"))
        && let Some(real) = rest.next()
    {
        buildscript::main(PathBuf::from(real), rest.collect());
    }
    // Remote build plumbing: the local sync daemon and the agent it starts
    // on the remote machine. Internal, not part of the CLI.
    match std::env::args_os()
        .nth(1)
        .as_deref()
        .and_then(|a| a.to_str())
    {
        Some("__daemon") => remote_daemon::main(),
        Some("__agent") => remote_agent::main(std::env::args().nth(2)),
        _ => {}
    }
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
        Command::Status {
            json,
            waybar,
            watch,
        } => status::command(json, waybar, watch)?,
        Command::Split {
            days,
            top,
            runs,
            apply,
            name,
            dry_run,
        } => match apply {
            Some(target) => split_apply::command(&target, name.as_deref(), dry_run)?,
            None => split::command(days, top, &runs)?,
        },
        Command::Slots {
            clean,
            gc,
            du,
            gc_root,
        } => slots::command(clean, gc, du, gc_root)?,
        Command::Cache { prune, clear, auto } => depcache_gc::command(prune, clear, auto)?,
        Command::Install { dir } => install::install(dir)?,
        Command::Uninstall { dir } => install::uninstall(dir)?,
        Command::Remote { cmd } => match cmd {
            RemoteCmd::Up {
                region,
                instance_type,
                on_demand,
                idle_minutes,
            } => remote::up(remote::UpOptions {
                region,
                instance_type,
                on_demand,
                idle_minutes,
            })?,
            RemoteCmd::Down => remote::down()?,
            RemoteCmd::Destroy { yes } => {
                if !yes {
                    anyhow::bail!("this terminates the machine and deletes its disk. Pass --yes");
                }
                remote::destroy()?
            }
            RemoteCmd::Status {
                json,
                waybar,
                watch,
            } => remote::status(json, waybar, watch)?,
            RemoteCmd::Ssh { cmd } => remote::ssh(cmd)?,
            RemoteCmd::Check(a) => remote_build::run("check", a.args)?,
            RemoteCmd::Test(a) => remote_build::run("test", a.args)?,
            RemoteCmd::Build(a) => remote_build::run("build", a.args)?,
            RemoteCmd::Clippy(a) => remote_build::run("clippy", a.args)?,
            RemoteCmd::Use {
                kind,
                host,
                port,
                identity,
            } => remote_backend::use_command(&kind, host, port, identity)?,
        },
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
