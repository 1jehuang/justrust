//! justrust: Rust compile as a service for coding agents.
//!
//! One binary, three entry points chosen by the name it is run as:
//! - `cargo` (via `justrust install`): records the build, then behaves like cargo.
//! - `rustc` (the shim cargo uses during a recorded run): times each unit.
//! - `justrust`: the CLI below. `justrust check|test|build|clippy|run` is the
//!   interface agents should use. It takes the same arguments as cargo.
//!
//! Linux, macOS, and Windows. Elsewhere `src/main.rs` runs cargo unchanged.
#![cfg(any(target_os = "linux", target_os = "macos", windows))]

mod account;
mod agent_output;
mod agents;
mod buildd;
mod buildscript;
mod depcache;
mod depcache_gc;
mod depcache_prebuilt;
mod depcache_remote;
mod early;
mod export;
mod findings;
mod history;
#[cfg(unix)]
mod hotpatch;
/// Hot patching rewrites ELF binaries: nothing to do on Windows.
#[cfg(windows)]
mod hotpatch {
    use std::ffi::OsString;
    pub fn enabled() -> bool {
        false
    }
    pub fn rustc_args(_local: bool, _test: bool, _linker_args: bool) -> Vec<OsString> {
        Vec::new()
    }
    pub fn link_env() -> (&'static str, &'static str) {
        ("JUSTRUST_HOTPATCH_LINK", "1")
    }
    pub fn before_link(_raw: &[OsString]) -> Option<i32> {
        None
    }
    pub fn after_link(_raw: &[OsString]) {}
}
mod install;
mod linker;
mod live;
mod localcache;
mod log_journal;
mod mcp;
mod paths;
mod peercache;
mod procfs;
mod record;
mod remote;
#[cfg(unix)]
mod remote_agent;
/// The remote agent runs on the (Linux) build machines only.
#[cfg(windows)]
mod remote_agent {
    pub const CLIENT_CARGO_HOME: &str = "JUSTRUST_CLIENT_CARGO_HOME";
    pub fn main(_prefix: Option<String>) -> ! {
        eprintln!("justrust: the remote build agent runs on Linux machines only");
        std::process::exit(2);
    }
}
mod remote_artifacts;
mod remote_backend;
mod remote_build;
mod remote_daemon;
mod remote_hosted;
// Parts are used only by the remote agent, which is not built on Windows.
mod feedback;
#[cfg_attr(windows, allow(dead_code))]
mod remote_proto;
mod remote_sync;
// Windows has no watcher backend yet (see the module docs).
#[cfg_attr(windows, allow(dead_code))]
mod remote_watch;
mod route;
mod runs;
mod sched;
mod shadow;
mod shim;
mod slots;
mod split;
mod split_apply;
mod split_index;
mod status;
mod summary;
mod sys;
mod telemetry;
mod test_pool;
mod test_runner;
mod toolchain;
mod update;

use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::path::PathBuf;

const AFTER_HELP: &str = "\
Agents: use `justrust check`, `justrust test`, `justrust build`, `justrust clippy`,
and `justrust run` instead of the matching cargo commands. They take exactly the
same arguments as cargo, record a timing profile, and print compact output:
errors and failing tests in full, the first few warnings, no progress noise,
then a one-line verdict. The full output is always saved: `justrust log`.

Where check/clippy/test run: signed in to hosted builds (API key or
subscription) means remote by default. Choose per command with `--local`,
`--remote`, or `--auto` (the router: remote only when predicted faster),
or set JUSTRUST_REMOTE=off|force|auto.

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
        /// (`JUSTRUST_DEPCACHE_MAX`, default a quarter of free disk, 30G to 150G).
        #[arg(long)]
        prune: bool,
        /// Remove every entry not used in the last 10 minutes.
        #[arg(long)]
        clear: bool,
        /// Background collection after a run (internal).
        #[arg(long, hide = true)]
        auto: bool,
        /// Upload queued entries to the remote cache now (runs in the
        /// background after builds on its own).
        #[arg(long)]
        sync: bool,
        /// Copy every dependency entry of the local cache into a prebuilt
        /// store directory (`<dir>/v1/...`, served read-only to
        /// `JUSTRUST_PREBUILT`). For the prebuild fleet.
        #[arg(long, value_name = "DIR")]
        publish: Option<PathBuf>,
    },
    /// Early type errors from a resident rust-analyzer (opt-in with
    /// JUSTRUST_EARLY=1): start, stop, or status of this workspace's daemon.
    Early {
        /// start | stop | status
        #[arg(default_value = "status")]
        action: String,
    },
    /// Resident build daemon (opt-in with JUSTRUST_BUILDD=1): status,
    /// start, stop, or `prebuild -- check -p x` to start a build early.
    Daemon {
        /// status | start | stop | prebuild
        #[arg(default_value = "status")]
        action: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<OsString>,
    },
    /// Pinned Rust toolchain (justrust.toml `[toolchain] channel`):
    /// status, install, pin, unpin, remove.
    Toolchain {
        #[command(subcommand)]
        cmd: Option<ToolchainCmd>,
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
    /// Modules `JUSTRUST_SHADOW=1` can route to a shadow leaf crate, ranked
    /// by recorded edits in this workspace.
    Shadow {
        #[arg(long, default_value_t = 30.0)]
        days: f64,
        #[arg(long, default_value_t = 15)]
        top: usize,
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
        /// Recompute the per-file split hints builds show (normally run in
        /// the background after builds that edited files).
        #[arg(long, hide = true, conflicts_with = "apply")]
        refresh_hints: bool,
    },
    /// Sign in to Jcode for hosted remote builds (device flow in the
    /// browser). Works without a TTY: prints the URL to relay to the user.
    Login {
        /// Print the URL and exit. Later justrust commands finish the
        /// sign-in once it is approved.
        #[arg(long)]
        no_wait: bool,
    },
    /// Remove the Jcode API key (keeps other settings in the file).
    Logout,
    /// MCP server on stdio for the Jcode account: status, sign-in, API keys,
    /// billing links. `claude mcp add justrust -- justrust mcp`.
    Mcp,
    /// Tell installed coding agents (Claude Code, Codex, Gemini CLI, Jcode)
    /// to use justrust: adds a short managed block to their global
    /// instruction file. Safe to rerun.
    Agents {
        /// Take the block out again.
        #[arg(long)]
        remove: bool,
        /// Show what would change.
        #[arg(long)]
        dry_run: bool,
        /// Only report changes.
        #[arg(long, hide = true)]
        quiet: bool,
    },
    /// Subscribe for hosted builds (first month of $20 free with a card), or
    /// switch an existing plan: to the next tier, or to `--plan N`.
    Upgrade {
        /// Plan in USD per month (multiples of $10, $20 to $1,000). Default:
        /// $20 for a new subscription, the next tier up for subscribers.
        #[arg(long)]
        plan: Option<u32>,
        /// Print the checkout URL and exit. Remote builds resume by
        /// themselves once paid.
        #[arg(long)]
        no_wait: bool,
    },
    /// Experimental: the remote compile machine (create, start, stop, status, ssh).
    Remote {
        #[command(subcommand)]
        cmd: RemoteCmd,
    },
    /// Append slim, path-redacted run summaries to the repo's data/runs/<machine>/.
    Export {
        /// Output root. Defaults to ./data/runs in the justrust repository.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Machine directory name. Defaults to the hostname.
        #[arg(long)]
        machine: Option<String>,
        /// Report what would be added without writing.
        #[arg(long)]
        dry_run: bool,
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
    /// Update justrust now, or show/change automatic updates
    /// (`status`, `on`, `off`). Release binaries update themselves daily.
    Update {
        /// now (default) | status | on | off
        #[arg(default_value = "now")]
        action: String,
        /// Update even while a build runs, or replace a source build.
        #[arg(long)]
        force: bool,
    },
    /// Send an explicit message to maintainers (max 4096 UTF-8 bytes).
    /// Do not include secrets or private paths. Respects telemetry opt-out.
    Feedback {
        /// Message to send. Quote it as one argument.
        message: String,
    },
    /// Anonymous usage stats: status (default), on, off.
    /// Also off with JUSTRUST_NO_TELEMETRY=1 or DO_NOT_TRACK=1.
    Telemetry {
        /// status | on | off
        #[arg(default_value = "status")]
        action: String,
    },
}

#[derive(Subcommand)]
enum ToolchainCmd {
    /// Which toolchain builds here use, and what is installed (default).
    Status,
    /// Download a toolchain (default: the one pinned here) from
    /// static.rust-lang.org into ~/.justrust/toolchains.
    Install {
        /// nightly-YYYY-MM-DD, beta-YYYY-MM-DD, X.Y.Z, or `nightly` (latest
        /// complete).
        spec: Option<String>,
    },
    /// Pin this project (writes justrust.toml) and install it.
    Pin {
        /// nightly-YYYY-MM-DD, X.Y.Z, or `nightly` (latest complete).
        spec: String,
        #[arg(long)]
        no_install: bool,
    },
    /// Remove the pin: builds here use the system toolchain again.
    Unpin,
    /// Delete an installed toolchain.
    Remove { spec: String },
    /// Install a jr-cargo build (justrust's cargo fork) into a toolchain:
    /// builds with that toolchain then run it instead of stock cargo
    /// (`JUSTRUST_CARGO_FORK=0` turns it off). Must be built from the
    /// cargo commit the toolchain ships.
    CargoFork {
        /// Path to the built binary, e.g. ~/jr-cargo/target/release/cargo.
        binary: PathBuf,
        /// Toolchain (default: the one pinned here, or the only installed one).
        #[arg(long)]
        toolchain: Option<String>,
    },
    /// Download the wild linker (pinned release, checksum verified). Local
    /// crates (test binaries, bins) then link with it, falling back to the
    /// configured linker if wild fails. `JUSTRUST_LINKER=off` disables it.
    Wild,
    /// Install a jr-rustc build (justrust's rustc fork) as toolchain
    /// `jr-<commit>`. With no pin, builds then use the newest installed jr
    /// toolchain and its proc-macro cache (`JUSTRUST_TOOLCHAIN=system` or
    /// `JUSTRUST_CACHE_PROC_MACROS=0` to opt out).
    JrInstall {
        /// Sysroot with bin/{rustc,rustdoc,clippy-driver,cargo-clippy}.
        #[arg(default_value = "~/jr-rustc/build/host/stage2")]
        sysroot: String,
        /// Installed official toolchain to take cargo and rustfmt from.
        #[arg(long, default_value = "nightly-2026-10-08")]
        cargo_from: String,
        /// Stage for explicit pins only, without changing the default compiler.
        #[arg(long)]
        no_default: bool,
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
    /// Choose where remote builds run: hosted (Jcode account, see
    /// `justrust login`), aws (legacy: the machine `remote up` creates in
    /// your AWS account), or off.
    Use { kind: String },
    /// Shell assignments for scripts (`eval "$(justrust remote env)"`):
    /// JR_REMOTE_KIND, JR_REMOTE_DEST, JR_REMOTE_SSH (ssh with the shared
    /// connection's options, also usable as `rsync -e`). Starts the machine
    /// if needed.
    Env,
}

#[derive(clap::Args)]
struct RemoteArgs {
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    args: Vec<String>,
}

/// `justrust <check|test|build|clippy|run|cargo> ARGS...` -> the subcommand
/// and ARGS exactly as given, `--help` included (cargo's help, as before).
fn passthrough_split(argv: &[OsString]) -> Option<(&'static str, Vec<OsString>)> {
    let sub = argv.get(1)?.to_str()?;
    let sub = ["check", "test", "build", "clippy", "run", "cargo"]
        .into_iter()
        .find(|s| *s == sub)?;
    Some((sub, argv[2..].to_vec()))
}

/// `--local` / `--remote` / `--auto` before any `--`: aliases for
/// JUSTRUST_REMOTE=off|force|auto (the router).
fn take_route_flags(args: Vec<OsString>) -> (Vec<OsString>, Option<&'static str>) {
    let mut mode = None;
    let mut out = Vec::with_capacity(args.len());
    let mut passed = false;
    for a in args {
        passed |= a == "--";
        match a.to_str() {
            Some("--local") if !passed => mode = Some("off"),
            Some("--remote") if !passed => mode = Some("force"),
            Some("--auto") if !passed => mode = Some("auto"),
            _ => out.push(a),
        }
    }
    (out, mode)
}

/// Remote routing must precede local early answers. A resident prebuild or
/// shadow result must not override an explicit or default hosted route.
fn route_then_accelerate(
    full: &[OsString],
    route: impl FnOnce(&[OsString]),
    daemon: impl FnOnce(&[OsString]) -> Option<i32>,
    shadow: impl FnOnce(&[OsString]),
) -> Option<i32> {
    route(full);
    if let Some(code) = daemon(full) {
        return Some(code);
    }
    shadow(full);
    None
}

fn agent(sub: &str, args: Vec<OsString>) -> ! {
    let (args, route_mode) = take_route_flags(args);
    if let Some(m) = route_mode {
        // SAFETY: single-threaded here, before any build starts.
        unsafe { std::env::set_var("JUSTRUST_REMOTE", m) };
    }
    let mut full = vec![OsString::from(sub)];
    full.extend(args);
    // Hosted/forced remote runs exit before a local daemon can return a
    // prebuild. Only a chosen local build may use local early answers.
    if let Some(code) = route_then_accelerate(
        &full,
        route::maybe_remote,
        buildd::maybe_run,
        shadow::maybe_route,
    ) {
        std::process::exit(code);
    }
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

pub fn main() -> anyhow::Result<()> {
    let argv0 = std::env::args_os().next().unwrap_or_default();
    let name = sys::program_name(&argv0);
    // `justrust __build-script <real> ...`: the build-script run wrapper.
    let mut rest = std::env::args_os().skip(1);
    if rest.next().as_deref() == Some(std::ffi::OsStr::new("__build-script"))
        && let Some(real) = rest.next()
    {
        buildscript::main(PathBuf::from(real), rest.collect());
    }
    // `justrust __test-runner <binary> ...`: cargo's runner during `test`.
    if std::env::args_os().nth(1).as_deref() == Some(std::ffi::OsStr::new(test_runner::SUBCOMMAND))
    {
        test_runner::main(std::env::args_os().skip(2));
    }
    // Remote build plumbing: the local sync daemon and the agent it starts
    // on the remote machine. Internal, not part of the CLI.
    match std::env::args_os()
        .nth(1)
        .as_deref()
        .and_then(|a| a.to_str())
    {
        Some("__daemon") => remote_daemon::main(),
        Some(early::DAEMON_ARG) => early::daemon_main(
            std::env::args_os()
                .nth(2)
                .map(PathBuf::from)
                .unwrap_or_default(),
        ),
        Some(buildd::DAEMON_ARG) => buildd::daemon_main(),
        Some(telemetry::BG_ARG) => telemetry::background_main(),
        Some("__agent") => remote_agent::main(std::env::args().nth(2)),
        Some(remote_artifacts::ARG) => remote_artifacts::serve_main(),
        Some(localcache::FINISH_ARG) => localcache::finish_main(
            std::env::args_os()
                .nth(2)
                .map(PathBuf::from)
                .unwrap_or_default(),
        ),
        _ => {}
    }
    if linker::invoked_as_ld(&argv0) {
        linker::ld_main();
    }
    if shim::invoked_as_shim(&argv0) {
        shim::main();
    }
    telemetry::install_panic_hook();
    if name == "cargo" {
        record::main(std::env::args_os().skip(1).collect());
    }
    update::notices();

    // Cargo passthrough subcommands take argv verbatim: clap swallows a
    // leading `--` (`justrust test -- --skip x` reached cargo as
    // `test --skip x` and failed).
    let argv: Vec<OsString> = std::env::args_os().collect();
    if let Some((sub, rest)) = passthrough_split(&argv) {
        account::poll_pending();
        if sub == "cargo" {
            record::main(rest);
        }
        agent(sub, rest);
    }

    let cli = Cli::parse();
    if !matches!(
        cli.command,
        Command::Login { .. } | Command::Logout | Command::Mcp | Command::Feedback { .. }
    ) {
        account::poll_pending();
    }
    let is_feedback = matches!(cli.command, Command::Feedback { .. });
    let name = command_name(&cli.command);
    let started = std::time::Instant::now();
    let result = dispatch(cli.command);
    if let Some(name) = name {
        telemetry::record(
            "command",
            serde_json::json!({
                "name": name,
                "ok": result.is_ok(),
                "secs": (started.elapsed().as_secs_f64() * 100.0).round() / 100.0,
            }),
        );
    }
    if !is_feedback {
        telemetry::maybe_spawn_background();
    }
    result
}

/// The subcommand's name for telemetry (no arguments). `None` for the
/// build commands (recorded as runs) and for `telemetry` itself.
fn command_name(c: &Command) -> Option<&'static str> {
    Some(match c {
        Command::Check(_)
        | Command::Test(_)
        | Command::Build(_)
        | Command::Clippy(_)
        | Command::Run(_)
        | Command::Cargo(_)
        | Command::Feedback { .. }
        | Command::Telemetry { .. } => return None,
        Command::Log { .. } => "log",
        Command::Runs { .. } => "runs",
        Command::Status { watch: Some(_), .. } => return None,
        Command::Status { .. } => "status",
        Command::Show { .. } => "show",
        Command::Slots {
            gc_root: Some(_), ..
        } => return None,
        Command::Slots { .. } => "slots",
        Command::Cache { auto: true, .. } => return None,
        Command::Cache { .. } => "cache",
        Command::Early { .. } => "early",
        Command::Daemon { .. } => "daemon",
        Command::Toolchain { .. } => "toolchain",
        Command::Install { .. } => "install",
        Command::Uninstall { .. } => "uninstall",
        Command::Shadow { .. } => "shadow",
        Command::Split {
            refresh_hints: true,
            ..
        } => return None,
        Command::Split { apply: Some(_), .. } => "split_apply",
        Command::Split { .. } => "split",
        Command::Login { .. } => "login",
        Command::Logout => "logout",
        Command::Mcp => "mcp",
        Command::Agents { .. } => "agents",
        Command::Upgrade { .. } => "upgrade",
        Command::Remote { .. } => "remote",
        Command::Export { .. } => "export",
        Command::History { .. } => "history",
        Command::Update { .. } => "update",
    })
}

fn dispatch(command: Command) -> anyhow::Result<()> {
    match command {
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
        Command::Shadow { days, top } => shadow::candidates_command(days, top)?,
        Command::Split {
            days,
            top,
            runs,
            apply,
            name,
            dry_run,
            refresh_hints,
        } => match apply {
            Some(target) => split_apply::command(&target, name.as_deref(), dry_run)?,
            None if refresh_hints => split::refresh_hints(days)?,
            None => split::command(days, top, &runs)?,
        },
        Command::Slots {
            clean,
            gc,
            du,
            gc_root,
        } => slots::command(clean, gc, du, gc_root)?,
        Command::Cache {
            prune,
            clear,
            auto,
            sync,
            publish,
        } => depcache_gc::command(prune, clear, auto, sync, publish)?,
        Command::Early { action } => early::command(&action)?,
        Command::Daemon { action, args } => buildd::command(&action, args)?,
        Command::Toolchain { cmd } => match cmd.unwrap_or(ToolchainCmd::Status) {
            ToolchainCmd::Status => toolchain::status()?,
            ToolchainCmd::Install { spec } => toolchain::install_command(spec)?,
            ToolchainCmd::Pin { spec, no_install } => toolchain::pin_command(&spec, no_install)?,
            ToolchainCmd::Unpin => toolchain::unpin_command()?,
            ToolchainCmd::Remove { spec } => toolchain::remove_command(&spec)?,
            ToolchainCmd::JrInstall {
                sysroot,
                cargo_from,
                no_default,
            } => {
                use anyhow::Context as _;
                let sysroot = match sysroot.strip_prefix("~/") {
                    Some(rest) => dirs::home_dir().context("no home")?.join(rest),
                    None => PathBuf::from(sysroot),
                };
                let from = toolchain::installed(&toolchain::Spec::parse(&cargo_from)?)?
                    .with_context(|| format!("{cargo_from} is not installed (`justrust toolchain install {cargo_from}`)"))?;
                let tc = toolchain::jr_install(&sysroot, &from, !no_default)?;
                println!("installed {} at {}", tc.id, tc.dir.display());
            }
            ToolchainCmd::CargoFork { binary, toolchain } => {
                toolchain::cargo_fork_command(&binary, toolchain)?
            }
            ToolchainCmd::Wild => linker::install_command()?,
        },
        Command::Install { dir } => install::install(dir)?,
        Command::Uninstall { dir } => install::uninstall(dir)?,
        Command::Login { no_wait } => account::login(no_wait)?,
        Command::Logout => account::logout()?,
        Command::Mcp => mcp::serve()?,
        Command::Agents {
            remove,
            dry_run,
            quiet,
        } => agents::command(remove, dry_run, quiet)?,
        Command::Upgrade { plan, no_wait } => account::upgrade(plan, no_wait)?,
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
            RemoteCmd::Use { kind } => remote_backend::use_command(&kind)?,
            RemoteCmd::Env => remote_backend::env_command()?,
        },
        Command::Export {
            out,
            machine,
            dry_run,
        } => export::command(out, machine, dry_run)?,
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
        Command::Update { action, force } => update::command(&action, force)?,
        Command::Feedback { message } => feedback::command(&message)?,
        Command::Telemetry { action } => telemetry::command(&action)?,
    }
    Ok(())
}

#[cfg(test)]
mod passthrough_tests {
    use super::*;

    fn split(args: &[&str]) -> Option<(&'static str, Vec<String>)> {
        let argv: Vec<OsString> = args.iter().map(OsString::from).collect();
        passthrough_split(&argv)
            .map(|(s, r)| (s, r.into_iter().map(|a| a.into_string().unwrap()).collect()))
    }

    #[test]
    fn routing_precedes_a_daemon_prebuild_answer() {
        let calls = std::cell::RefCell::new(Vec::new());
        let args = [OsString::from("check")];
        let code = route_then_accelerate(
            &args,
            |full| {
                assert_eq!(full, args);
                calls.borrow_mut().push("route");
            },
            |_| {
                calls.borrow_mut().push("daemon");
                Some(23)
            },
            |_| calls.borrow_mut().push("shadow"),
        );
        assert_eq!(code, Some(23));
        assert_eq!(*calls.borrow(), ["route", "daemon"]);
    }

    #[test]
    fn local_fallback_keeps_daemon_then_shadow_order_and_arguments() {
        let calls = std::cell::RefCell::new(Vec::new());
        let args: Vec<OsString> = ["test", "--lib", "--", "fps_counter"]
            .into_iter()
            .map(OsString::from)
            .collect();
        let code = route_then_accelerate(
            &args,
            |full| {
                assert_eq!(full, args);
                calls.borrow_mut().push("route");
            },
            |full| {
                assert_eq!(full, args);
                calls.borrow_mut().push("daemon");
                None
            },
            |full| {
                assert_eq!(full, args);
                calls.borrow_mut().push("shadow");
            },
        );
        assert_eq!(code, None);
        assert_eq!(*calls.borrow(), ["route", "daemon", "shadow"]);
    }

    #[test]
    fn leading_double_dash_reaches_cargo() {
        let (sub, rest) = split(&["justrust", "test", "--", "--skip", "x"]).unwrap();
        assert_eq!(sub, "test");
        assert_eq!(rest, ["--", "--skip", "x"]);
        let (_, rest) = split(&["justrust", "test", "--lib", "--", "--nocapture"]).unwrap();
        assert_eq!(rest, ["--lib", "--", "--nocapture"]);
    }

    #[test]
    fn other_subcommands_go_to_clap() {
        assert!(split(&["justrust", "show"]).is_none());
        assert!(split(&["justrust"]).is_none());
        assert_eq!(
            split(&["justrust", "check", "--help"]).unwrap().1,
            ["--help"]
        );
    }
}
