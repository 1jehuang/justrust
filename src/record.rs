//! `justrust cargo ...`: run the real cargo and record where the time went.
//!
//! The recorder:
//! - runs the real cargo with `RUSTC` pointed at the justrust shim, so every
//!   compilation unit reports wall, CPU, memory, and pass timings,
//! - tees stdout and stderr byte for byte, logging each line with a timestamp,
//! - samples system CPU, memory, pressure stalls, and the build's process tree,
//! - writes `meta.json`, `summary.json`, and one line in `index.jsonl`.
//!
//! It fails open: if recording cannot be set up, it execs cargo unchanged.

use crate::{agent_output, paths, procfs, slots, summary};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cargo subcommands worth recording. Everything else (metadata, fmt, tree,
/// --version, ...) is passed straight through with no overhead.
const RECORDED: &[&str] = &[
    "build", "b", "check", "c", "test", "t", "bench", "run", "r", "clippy", "doc", "d", "rustc",
    "rustdoc", "nextest", "fix", "miri", "llvm-cov", "insta", "zigbuild",
];

const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);
const INITIAL_SAMPLE_INTERVAL: Duration = Duration::from_millis(50);
const INITIAL_SAMPLE_WINDOW: Duration = Duration::from_millis(400);

fn sample_interval(elapsed: Duration) -> Duration {
    if elapsed < INITIAL_SAMPLE_WINDOW {
        INITIAL_SAMPLE_INTERVAL
    } else {
        SAMPLE_INTERVAL
    }
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct Meta {
    pub id: String,
    pub args: Vec<String>,
    pub subcommand: String,
    pub cwd: String,
    pub start: f64,
    pub pid: u32,
    pub real_cargo: String,
    pub real_rustc: String,
    pub rustc_wrapper: Option<String>,
    pub agent_session: Option<String>,
    pub tty: bool,
    pub ncpu: usize,
    pub mem_total_mb: u64,
    pub env: HashMap<String, String>,
    /// Per-agent build slot used for this run (see `slots`).
    #[serde(default)]
    pub slot: Option<slots::SlotInfo>,
    /// Pinned toolchain id (`justrust.toml`), `None` for the system one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub toolchain: Option<String>,
    /// Why this agent-mode build ran here and not on the remote machine
    /// (`route::local_reason`), `None` when routing was not consulted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Where the build runs: "local", "remote" (own machine) or "hosted".
    /// `None` in runs recorded before this existed (they were local).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub place: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GitInfo {
    pub root: Option<String>,
    pub head: Option<String>,
    pub dirty_files: Option<usize>,
    /// Total added + deleted lines in modified `.rs` files relative to HEAD.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty_rs_lines: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Sample {
    pub t: f64,
    /// Whole-machine busy CPUs (cores' worth) since the previous sample.
    pub sys_cores: f64,
    /// Cores' worth used by this build's process tree since the previous sample.
    pub build_cores: f64,
    pub build_rss_mb: f64,
    pub mem_available_mb: u64,
    pub rustc_procs: usize,
    /// rustc processes running on the machine that are not part of this build.
    pub foreign_rustc_procs: usize,
    pub psi_cpu_ms: f64,
    pub psi_mem_ms: f64,
    pub psi_io_ms: f64,
    /// The machine has no pressure stall info (macOS, a kernel without
    /// PSI): the `psi_*` fields are not measured, not zero.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub psi_unavailable: bool,
}

/// Live, interval CPU usage. This is observational run metadata, never an
/// input to Cargo or rustc and never a process-count estimate.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct LiveCpu {
    pub t: f64,
    pub build_cores: f64,
    #[serde(default)]
    pub ncpu: usize,
}

pub(crate) fn write_live_cpu(dir: &Path, cpu: &LiveCpu) {
    if let Ok(bytes) = serde_json::to_vec(cpu) {
        let tmp = dir.join("cpu.json.tmp");
        if std::fs::write(&tmp, bytes).is_ok() {
            let _ = std::fs::rename(tmp, dir.join("cpu.json"));
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ProcRecord {
    pub pid: i32,
    pub comm: String,
    pub cmdline: String,
    pub kind: String,
    pub first_seen: f64,
    pub last_seen: f64,
    pub cpu_secs: f64,
    pub max_rss_mb: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OutputLine {
    pub t: f64,
    /// "o" for stdout, "e" for stderr.
    pub s: String,
    pub l: String,
}

struct OutputCapture {
    lines: Vec<OutputLine>,
    journal: crate::log_journal::Journal,
}

static CHILD_PID: AtomicI32 = AtomicI32::new(0);

#[cfg(unix)]
extern "C" fn forward_signal(sig: libc::c_int) {
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 && sig != libc::SIGINT {
        // SAFETY: kill is async-signal-safe.
        unsafe {
            libc::kill(pid, sig);
        }
    }
    crate::test_pool::signal_live(sig);
}

#[cfg(unix)]
fn install_signal_forwarding() {
    // SAFETY: installing a handler that only calls async-signal-safe functions.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = forward_signal as *const () as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT, libc::SIGINT] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
        // The terminal already delivers SIGINT to cargo. The handler marks
        // cancellation and signals pooled tests without interrupting summary
        // writing or allowing confirmation retries after Ctrl-C.
    }
}

/// Windows: the console delivers Ctrl+C and Ctrl+Break to cargo and the
/// test processes too (one console); ignore them here so the summary still
/// gets written. Closing the console ends the whole tree anyway.
#[cfg(windows)]
fn install_signal_forwarding() {
    crate::sys::ignore_ctrl_c();
}

/// Find the cargo subcommand, skipping `+toolchain` and global options.
pub fn subcommand(args: &[OsString]) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].to_string_lossy();
        if a.starts_with('+') {
            i += 1;
            continue;
        }
        if matches!(
            a.as_ref(),
            "--color" | "--config" | "-Z" | "-C" | "--explain"
        ) {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        return Some(a.into_owned());
    }
    None
}

pub fn should_record(args: &[OsString]) -> bool {
    if std::env::var_os("JUSTRUST_DISABLE").is_some_and(|v| v != "0") {
        return false;
    }
    // Nested cargo (from a build script, test, or cargo plugin): the outer run
    // already records it.
    if std::env::var_os("JUSTRUST_RUN_DIR").is_some() {
        return false;
    }
    subcommand(args).is_some_and(|s| RECORDED.contains(&s.as_str()))
}

pub fn exec_cargo(args: &[OsString]) -> ! {
    // Unrecorded commands (fmt, metadata, tree) use a pinned toolchain only
    // when it is already installed: they never trigger a download.
    let selection = match crate::toolchain::slots::selection() {
        Ok(lock) => lock,
        Err(e) => {
            eprintln!("justrust: compiler admission: {e:#}");
            std::process::exit(1);
        }
    };
    let pinned = std::env::current_dir()
        .ok()
        .and_then(|d| crate::toolchain::resolve(&d).ok().flatten())
        .and_then(|p| crate::toolchain::available_for_build(&p.spec));
    let pinned = match compatible_toolchain(pinned, args) {
        Ok(tc) => tc,
        Err(e) => {
            eprintln!("justrust: {e:#}");
            std::process::exit(1);
        }
    };
    if matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_REQUIRED").as_deref(),
        Ok("1" | "true" | "yes")
    ) && subcommand(args).is_some_and(|s| RECORDED.contains(&s.as_str()))
        && pinned.is_none()
    {
        eprintln!(
            "justrust: toolchain required: no installed explicit pin, refusing unrecorded system compiler fallback"
        );
        report_build_failure(args, "toolchain_required", false, Some(1));
        std::process::exit(1);
    }
    let _compiler_lease = match crate::toolchain::slots::lease(pinned.as_ref()) {
        Ok(lease) => lease,
        Err(e) => {
            eprintln!("justrust: compiler admission: {e:#}");
            std::process::exit(1);
        }
    };
    let mut cmd;
    match &pinned {
        Some(tc) => {
            cmd = Command::new(tc.cargo());
            apply_toolchain_env(&mut cmd, tc);
        }
        None => {
            cmd = Command::new(
                paths::real_cargo().unwrap_or_else(|_| PathBuf::from("/usr/bin/cargo")),
            );
        }
    }
    if subcommand(args).as_deref() == Some("zigbuild") && cfg!(target_os = "linux") {
        let result = crate::release_tools::ensure()
            .and_then(|tools| crate::release_tools::apply(&mut cmd, tools));
        if let Err(error) = result {
            eprintln!("justrust: release tools unavailable: {error:#}");
            report_build_failure(args, "toolchain_required", false, Some(1));
            std::process::exit(1);
        }
    }
    if let Err(error) = apply_musl_linker(&mut cmd, pinned.as_ref(), args) {
        eprintln!("justrust: {error:#}");
        report_build_failure(args, "toolchain_required", false, Some(1));
        std::process::exit(1);
    }
    if let Err(error) = apply_cross_tools(&mut cmd, pinned.as_ref(), args) {
        eprintln!("justrust: {error:#}");
        report_build_failure(args, "toolchain_required", false, Some(1));
        std::process::exit(1);
    }
    let mut platform_args = args.to_vec();
    if let Err(error) = apply_platform_tools(&mut cmd, &mut platform_args) {
        eprintln!("justrust: {error:#}");
        report_build_failure(args, "toolchain_required", false, Some(1));
        std::process::exit(1);
    }
    if let Some(lease) = &_compiler_lease {
        lease.inherit(&mut cmd);
    }
    let cargo = PathBuf::from(cmd.get_program());
    drop(selection);
    let err = crate::sys::exec(cmd.args(&platform_args));
    eprintln!("justrust: failed to exec {}: {err}", cargo.display());
    report_build_failure(args, "cargo_exec", false, Some(127));
    std::process::exit(127);
}

/// Initialization errors otherwise fall through to an unrecorded cargo exec.
/// Fixed labels only: error chains can contain source, credentials and paths.
fn build_failure_data(
    args: &[OsString],
    stage: &'static str,
    fallback: bool,
    exit: Option<i32>,
) -> serde_json::Value {
    let sub = subcommand(args)
        .filter(|s| RECORDED.contains(&s.as_str()))
        .unwrap_or_else(|| "other".to_string());
    serde_json::json!({ "schema_version": 2, "sub": sub, "stage": stage,
        "fallback": fallback, "exit": exit })
}

fn report_build_failure(args: &[OsString], stage: &'static str, fallback: bool, exit: Option<i32>) {
    crate::telemetry::record(
        "build_failure",
        build_failure_data(args, stage, fallback, exit),
    );
    crate::telemetry::maybe_spawn_background();
}

/// How a recorded run presents its output.
#[derive(Debug, Clone, Copy)]
pub struct Options {
    /// Filter output for coding agents (see `agent_output`) and end with a
    /// compact status block instead of the one-line footer.
    pub agent: bool,
    pub max_warnings: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            agent: false,
            max_warnings: agent_output::DEFAULT_MAX_WARNINGS,
        }
    }
}

/// Entry point for `justrust cargo ...` and the `cargo` proxy.
pub fn main(args: Vec<OsString>) -> ! {
    run(args, Options::default())
}

/// Record `cargo <args>` and exit with cargo's exit code.
pub fn run(args: Vec<OsString>, opts: Options) -> ! {
    crate::e2e::mark("routed");
    // The build daemon's note is for this process only: out of the
    // environment before cargo or a compiler can inherit it.
    crate::buildd::waited_prebuild();
    crate::buildd::build_stamp();
    let mut trace = PhaseTrace::new();
    if !should_record(&args) {
        exec_cargo(&args);
    }
    let id = new_run_id();
    let mut lifecycle = crate::telemetry::BuildLifecycle::start(
        &id,
        &subcommand(&args).unwrap_or_default(),
        crate::telemetry::local_route_label(),
    );
    lifecycle.report_local(&args);
    trace.mark("lifecycle-start");
    match record(&args, opts, &id, &mut lifecycle) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            if e.downcast_ref::<ToolchainRequired>().is_some() {
                eprintln!("justrust: {e:#}");
                report_build_failure(&args, "toolchain_required", false, Some(1));
                lifecycle.done(1, false);
                std::process::exit(1);
            }
            eprintln!("justrust: recording disabled for this run: {e:#}");
            report_build_failure(&args, "recording_init", true, None);
            drop(lifecycle);
            exec_cargo(&args);
        }
    }
}

pub fn new_run_id() -> String {
    // The remote agent records under the id the client chose, so the run
    // has the same id on both machines.
    if let Ok(id) = std::env::var("JUSTRUST_RUN_ID_OVERRIDE")
        && !id.is_empty()
        && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        // SAFETY: single-threaded at this point; keeps it out of cargo's env.
        unsafe { std::env::remove_var("JUSTRUST_RUN_ID_OVERRIDE") };
        return id;
    }
    let now = chrono::Local::now();
    format!("{}-{}", now.format("%Y%m%d-%H%M%S%.3f"), std::process::id()).replace('.', "")
}

fn isatty(fd: i32) -> bool {
    crate::sys::isatty(fd)
}

fn term_width(fd: i32) -> Option<u16> {
    crate::sys::term_width(fd)
}

/// Run cargo from a pinned toolchain: its `bin/` goes first on `PATH`, so
/// cargo finds the matching rustc, rustdoc, cargo-clippy, clippy-driver and
/// rustfmt, and so does anything cargo runs.
pub fn apply_toolchain_env(cmd: &mut Command, tc: &crate::toolchain::Toolchain) {
    let bin = tc.dir.join("bin");
    let mut dirs = vec![bin.clone()];
    if let Some(p) = std::env::var_os("PATH") {
        dirs.extend(std::env::split_paths(&p).filter(|d| d != &bin));
    }
    if let Ok(p) = std::env::join_paths(dirs) {
        cmd.env("PATH", p);
    }
    cmd.env("RUSTC", tc.bin("rustc"))
        .env("RUSTDOC", tc.bin("rustdoc"))
        .env("JUSTRUST_TOOLCHAIN_ACTIVE", &tc.id);
    // A rustup-managed environment must not redirect the pinned binaries.
    cmd.env_remove("RUSTUP_TOOLCHAIN");
}

/// Use the selected compiler's self-contained ELF linker for static musl
/// executables. Unlike Zig's driver this preserves rustc's Cortex-A53 fix.
fn apply_musl_linker(
    cmd: &mut Command,
    pinned: Option<&crate::toolchain::Toolchain>,
    args: &[OsString],
) -> Result<()> {
    apply_musl_linker_with_tools(cmd, pinned, args, crate::release_tools::ensure)
}

fn apply_musl_linker_with_tools(
    cmd: &mut Command,
    pinned: Option<&crate::toolchain::Toolchain>,
    args: &[OsString],
    tools: impl FnOnce() -> Result<Vec<PathBuf>>,
) -> Result<()> {
    if subcommand(args).as_deref() != Some("build") {
        return Ok(());
    }
    let targets = requested_targets(args, None);
    let [target] = targets.as_slice() else {
        return Ok(());
    };
    let Some(target) = target
        .to_str()
        .filter(|t| crate::remote_artifacts::musl_target(t))
    else {
        return Ok(());
    };
    let tc = pinned.ok_or_else(|| {
        ToolchainRequired("native musl builds require an explicit installed compiler pin".into())
    })?;
    let linker = tc
        .dir
        .join("lib/rustlib")
        .join(crate::toolchain::TARGET)
        .join("bin/rust-lld");
    if !linker.is_file() {
        return Err(
            ToolchainRequired(format!("pinned musl linker missing: {}", linker.display())).into(),
        );
    }
    let tools = tools()
        .map_err(|error| ToolchainRequired(format!("musl C tools unavailable: {error:#}")))?;
    crate::release_tools::apply_musl_c(cmd, target, &tools)?;
    cmd.env(
        format!(
            "CARGO_TARGET_{}_LINKER",
            target.replace('-', "_").to_ascii_uppercase()
        ),
        linker,
    )
    .env("JUSTRUST_LINKER", "off");
    Ok(())
}

/// Explicit cross targets must have libraries in the selected compiler's own
/// sysroot. Never borrow a system libcore for a different compiler version.
fn apply_cross_tools(
    cmd: &mut Command,
    pinned: Option<&crate::toolchain::Toolchain>,
    args: &[OsString],
) -> Result<()> {
    // A system-toolchain opt-out preserves the user's own linker configuration.
    if !cfg!(target_os = "linux") || pinned.is_none() {
        return Ok(());
    }
    if requested_targets(args, std::env::var_os("CARGO_BUILD_TARGET"))
        .iter()
        .any(|target| target.to_str().is_some_and(crate::cross_tools::supported))
    {
        let root = crate::cross_tools::ensure().map_err(|error| {
            ToolchainRequired(format!("Windows GNU tools unavailable: {error:#}"))
        })?;
        crate::cross_tools::apply(cmd, &root)?;
    }
    Ok(())
}

pub(crate) fn requested_targets(args: &[OsString], env_target: Option<OsString>) -> Vec<OsString> {
    let mut targets = Vec::new();
    let mut args = args.iter().take_while(|a| *a != "--");
    while let Some(arg) = args.next() {
        if arg == "--target" {
            if let Some(target) = args.next() {
                targets.push(target.clone());
            }
        } else if let Some(target) = arg.to_str().and_then(|s| s.strip_prefix("--target=")) {
            targets.push(target.into());
        }
    }
    if targets.is_empty()
        && let Some(target) = env_target
    {
        targets.push(target);
    }
    targets
}

fn apply_platform_tools(cmd: &mut Command, args: &mut Vec<OsString>) -> Result<()> {
    // These are Linux-hosted cross tools. Native platform builds keep their
    // existing SDK/compiler selection, including explicit MSVC targets.
    if !cfg!(target_os = "linux") {
        return Ok(());
    }
    let targets = requested_targets(args, None);
    let [target] = targets.as_slice() else {
        return Ok(());
    };
    let Some(target) = target.to_str() else {
        return Ok(());
    };
    match subcommand(args).as_deref() {
        Some("build") if crate::msvc_tools::supported(target) => {
            let root = crate::msvc_tools::prepare(target)?;
            crate::msvc_tools::apply(cmd, &root, target)?;
            let index = args
                .iter()
                .position(|arg| arg == "build")
                .context("missing build subcommand")?;
            args.insert(index, OsString::from("xwin"));
        }
        Some("zigbuild") if crate::darwin_tools::supported(target) => {
            let id = std::env::var("JUSTRUST_DARWIN_SDK").context(
                "macOS cross builds require JUSTRUST_DARWIN_SDK=<managed SDK id>; provision a licensed SDK on the host with justrust toolchain import-darwin-sdk --licensed <path>"
            )?;
            crate::darwin_tools::apply(cmd, target, &id)?;
        }
        Some("build") if crate::ios_tools::supported(target) => {
            // Without a managed SDK selection keep the caller's own iOS
            // linker setup (local builds); hosted builds always select one.
            if let Ok(id) = std::env::var("JUSTRUST_DARWIN_SDK") {
                crate::ios_tools::apply(cmd, &id)?;
            }
        }
        Some("build") if cfg!(target_arch = "x86_64") && crate::arm64_tools::supported(target) => {
            let text: Vec<String> = args
                .iter()
                .map(|arg| arg.to_string_lossy().into_owned())
                .collect();
            crate::arm64_tools::configure(cmd, &text)?;
        }
        _ => (),
    }
    Ok(())
}

fn target_has_core(rustc: &Path, target: &OsString) -> bool {
    let normalized = target.to_str().map(crate::remote_artifacts::rust_target);
    let Ok(output) = Command::new(rustc)
        .args(["--print", "target-libdir", "--target"])
        .arg(normalized.map(OsString::from).as_ref().unwrap_or(target))
        .output()
    else {
        return false;
    };
    if !output.status.success() {
        return false;
    }
    let dir = String::from_utf8_lossy(&output.stdout);
    std::fs::read_dir(dir.trim()).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with("libcore-") && (name.ends_with(".rlib") || name.ends_with(".rmeta"))
        })
    })
}

fn compatible_toolchain(
    pinned: Option<crate::toolchain::Toolchain>,
    args: &[OsString],
) -> Result<Option<crate::toolchain::Toolchain>> {
    let Some(tc) = pinned else { return Ok(None) };
    // build-std deliberately builds missing sysroot libraries with nightly.
    if args.iter().take_while(|a| *a != "--").any(|arg| {
        arg.to_str().is_some_and(|s| {
            s == "build-std"
                || s.starts_with("build-std=")
                || s == "-Zbuild-std"
                || s.starts_with("-Zbuild-std=")
        })
    }) {
        return Ok(Some(tc));
    }
    let targets = requested_targets(args, std::env::var_os("CARGO_BUILD_TARGET"));
    // Custom JSON targets may use build-std. Leave their policy to Cargo.
    // Zig release builds use the selected compiler's own official target
    // libraries, never the host's default rustup toolchain or borrowed std.
    if matches!(
        subcommand(args).as_deref(),
        Some("zigbuild" | "build" | "test" | "check" | "clippy")
    ) && !matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_AUTO_INSTALL").as_deref(),
        Ok("0" | "off" | "false")
    ) {
        for target in &targets {
            if let Some(target_name) = target.to_str()
                && (crate::remote_artifacts::musl_target(target_name)
                    || crate::remote_artifacts::glibc_target(target_name)
                    || crate::darwin_tools::supported(target_name)
                    // Unmanaged local iOS builds keep the caller's toolchain.
                    || (crate::ios_tools::supported(target_name)
                        && std::env::var_os("JUSTRUST_DARWIN_SDK").is_some())
                    || crate::arm64_tools::supported(target_name)
                    || crate::msvc_tools::supported(target_name)
                    || crate::cross_tools::supported(target_name))
                && !target_has_core(&tc.bin("rustc"), target)
                && let Err(error) =
                    crate::toolchain::add_target(&tc, target_name, &mut std::io::stderr())
            {
                return Err(ToolchainRequired(format!(
                    "could not install exact target libraries: {error:#}"
                ))
                .into());
            }
        }
    }
    let missing: Vec<_> = targets
        .iter()
        .filter(|t| !t.to_string_lossy().ends_with(".json"))
        .filter(|t| !target_has_core(&tc.bin("rustc"), t))
        .collect();
    if missing.is_empty() {
        return Ok(Some(tc));
    }
    let names = missing
        .iter()
        .map(|t| t.to_string_lossy())
        .collect::<Vec<_>>()
        .join(", ");
    if matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_REQUIRED").as_deref(),
        Ok("1" | "true" | "yes")
    ) {
        return Err(ToolchainRequired(format!("toolchain {} has no core library for {names}; install the target for this exact compiler or select a target-compatible JUSTRUST_TOOLCHAIN", tc.id)).into());
    }
    let me = std::env::current_exe()?.canonicalize()?;
    let system = std::env::var_os("RUSTC")
        .map(PathBuf::from)
        .filter(|p| p.canonicalize().ok().as_deref() != Some(me.as_path()))
        .or_else(|| paths::find_on_path("rustc", &me));
    if system.as_ref().is_some_and(|rustc| {
        targets
            .iter()
            .filter(|t| !t.to_string_lossy().ends_with(".json"))
            .all(|t| target_has_core(rustc, t))
    }) {
        eprintln!(
            "justrust: toolchain {} lacks target libraries for {names}; recording with the target-compatible system toolchain",
            tc.id
        );
        Ok(None)
    } else {
        // Preserve the compiler's original diagnostics when no compatible
        // fallback exists, rather than guessing or mixing sysroots.
        Ok(Some(tc))
    }
}

/// Bounded, opt-in wall-clock diagnostics. Never enters Cargo's environment or
/// artifact keys, and does no filesystem work when disabled.
struct PhaseTrace {
    last: std::time::Instant,
    enabled: bool,
}

impl PhaseTrace {
    fn new() -> Self {
        Self {
            last: std::time::Instant::now(),
            enabled: std::env::var_os("JUSTRUST_RECORD_PHASE_TRACE").as_deref()
                == Some(std::ffi::OsStr::new("1")),
        }
    }

    fn mark(&mut self, phase: &'static str) {
        let now = std::time::Instant::now();
        if self.enabled {
            eprintln!(
                "justrust: record-phase {phase} {:.3}ms",
                now.duration_since(self.last).as_secs_f64() * 1000.0
            );
        }
        self.last = now;
    }
}

fn record(
    args: &[OsString],
    opts: Options,
    id: &str,
    lifecycle: &mut crate::telemetry::BuildLifecycle,
) -> Result<i32> {
    let mut trace = PhaseTrace::new();
    // Git is observational metadata, not an input to Cargo or cache keys.
    // Start its independent filesystem walk before slot/setup work so a
    // no-op Cargo run does not pay that setup followed by the whole walk.
    let git = std::thread::spawn(git_info);
    // A pinned toolchain (`justrust.toml`) replaces the system cargo and
    // rustc. Installed on first use. Fails open to the system toolchain.
    let required = matches!(
        std::env::var("JUSTRUST_TOOLCHAIN_REQUIRED").as_deref(),
        Ok("1" | "true" | "yes")
    );
    let requested = if required {
        let cwd = std::env::current_dir()?;
        Some(crate::toolchain::resolve(&cwd)
            .map_err(|e| ToolchainRequired(format!("toolchain required: invalid pin: {e:#}")))?
            .ok_or_else(|| ToolchainRequired("toolchain required: set JUSTRUST_TOOLCHAIN to an exact version, not system".into()))?
            .spec.id())
    } else {
        None
    };
    let selection = crate::toolchain::slots::selection()?;
    let pinned = compatible_toolchain(crate::toolchain::for_build(), args)?;
    let _compiler_lease = crate::toolchain::slots::lease(pinned.as_ref())?;
    // Up to here the run may have waited for a toolchain lifecycle change on
    // this machine, or installed its pinned compiler.
    crate::e2e::mark("toolchain");
    require_selected_toolchain(
        requested.as_deref(),
        pinned.as_ref().map(|tc| tc.id.as_str()),
    )?;
    let real_cargo = match &pinned {
        Some(tc) => tc.cargo(),
        None => paths::real_cargo()?,
    };
    let me = std::env::current_exe()?.canonicalize()?;
    // Respect an existing RUSTC unless it already points at us.
    let real_rustc = match &pinned {
        Some(tc) => tc.bin("rustc"),
        None => std::env::var_os("RUSTC")
            .map(PathBuf::from)
            .filter(|p| p.canonicalize().ok().as_deref() != Some(me.as_path()))
            .or_else(|| paths::find_on_path("rustc", &me))
            .context("could not find rustc")?,
    };
    let shim = paths::ensure_rustc_shim_for(&real_rustc)?;
    trace.mark("toolchain");

    let run_dir = paths::runs_dir()?.join(id);
    std::fs::create_dir_all(&run_dir)?;

    // Moving into the systemd scope (busctl plus waiting for the move) and
    // resolving the slot (cargo metadata) are independent subprocess round
    // trips: overlap them. Cargo is spawned only after both finish, so it
    // still starts inside the scope.
    let scope_thread = {
        let id = id.to_owned();
        std::thread::spawn(move || crate::sched::enter(&id))
    };
    // Agent runs get a private target dir so parallel agents do not block.
    let mut slot = opts.agent.then(|| slots::acquire_for(args, id)).flatten();
    let cargo_args = slot.as_ref().map_or(args, |s| s.args.as_slice());
    trace.mark("slot");

    let tty = isatty(2);
    let env_keys = [
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_JOBS",
        "CARGO_INCREMENTAL",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_PROFILE_DEV_DEBUG",
        "JCODE_SESSION_ID",
        "JCODE_SCRATCH_DIR",
    ];
    let env = env_keys
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
        .collect();
    let meta = Meta {
        id: id.to_owned(),
        args: args
            .iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect(),
        subcommand: subcommand(args).unwrap_or_default(),
        cwd: std::env::current_dir()?.to_string_lossy().into_owned(),
        start: paths::now(),
        pid: std::process::id(),
        real_cargo: real_cargo.to_string_lossy().into_owned(),
        real_rustc: real_rustc.to_string_lossy().into_owned(),
        rustc_wrapper: std::env::var("RUSTC_WRAPPER").ok(),
        agent_session: std::env::var("JCODE_SESSION_ID").ok(),
        tty,
        ncpu: procfs::ncpu(),
        mem_total_mb: procfs::installed_mb().unwrap_or(0),
        env,
        slot: slot.as_ref().map(|s| s.info.clone()),
        toolchain: pinned.as_ref().map(|tc| tc.id.clone()),
        route: crate::route::local_reason(),
        place: Some("local".into()),
    };
    std::fs::write(run_dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)?;

    // Before cargo starts, so it and everything it runs share the scope.
    let scope = scope_thread.join().ok().flatten();

    // The progress bar feeds `live.json` (`justrust status`). When the user
    // would not have seen it, ask for it with `--config` (unlike the env var,
    // that does not leak into test binaries that run cargo) and strip it again.
    let user_progress = tty && !opts.agent;
    let strip_progress = !user_progress && std::env::var_os("CARGO_TERM_PROGRESS_WHEN").is_none();
    let mut cargo_args = if strip_progress {
        with_progress_config(cargo_args)
    } else {
        cargo_args.to_vec()
    };
    // Per-test durations (see `test_runner`). Runner config is not part of
    // any fingerprint, so artifacts stay shared with unrecorded builds.
    // In agent mode the runner hands binaries to a pool that runs them in
    // parallel (`test_pool`); the pool prints through the same tee.
    let lines = Arc::new(Mutex::new(OutputCapture {
        lines: Vec::new(),
        journal: crate::log_journal::Journal::new(&run_dir.join("output.jsonl")),
    }));
    let is_test = matches!(meta.subcommand.as_str(), "test" | "t" | "bench" | "nextest");
    let filter = |stdout: bool| {
        opts.agent.then(|| {
            Mutex::new(agent_output::Filter::new(
                stdout,
                is_test,
                opts.max_warnings,
            ))
        })
    };
    let f_out = Arc::new(filter(true));
    let f_err = Arc::new(filter(false));
    let out_lock: Arc<Mutex<()>> = Arc::default();
    let mut pool = None;
    if crate::test_runner::wanted(&meta.subcommand, &cargo_args) {
        if matches!(meta.subcommand.as_str(), "test" | "t") && crate::test_pool::enabled(opts.agent)
        {
            let sink = Arc::new(PoolSink {
                lines: lines.clone(),
                f_out: f_out.clone(),
                f_err: f_err.clone(),
                out_lock: out_lock.clone(),
            });
            let fail_fast = !cargo_args
                .iter()
                .take_while(|a| *a != "--")
                .any(|a| a == "--no-fail-fast");
            pool =
                crate::test_pool::Pool::start(&run_dir, Path::new(&meta.cwd), &me, sink, fail_fast);
        }
        let arg = pool.as_ref().map(|p| p.runner_arg());
        if let Some(c) = crate::test_runner::cargo_config(&me, arg.as_deref()) {
            cargo_args = insert_config(&cargo_args, &c);
        }
    }

    let mut cmd = Command::new(&real_cargo);
    if let Some(tc) = &pinned {
        apply_toolchain_env(&mut cmd, tc);
        if tc.nightly {
            // Lets the shim time passes of local crates: on nightly, unstable
            // options never change the incremental cache key.
            cmd.env("JUSTRUST_TOOLCHAIN_NIGHTLY", "1");
        }
        if tc.id.starts_with("jr-") {
            // justrust's compiler: the shim enables its caches.
            cmd.env("JUSTRUST_TOOLCHAIN_JR", "1");
        }
    }
    apply_musl_linker(&mut cmd, pinned.as_ref(), &cargo_args)?;
    apply_cross_tools(&mut cmd, pinned.as_ref(), &cargo_args)?;
    if meta.subcommand == "zigbuild" && cfg!(target_os = "linux") {
        let tools = crate::release_tools::ensure()
            .map_err(|error| ToolchainRequired(format!("release tools unavailable: {error:#}")))?;
        crate::release_tools::apply(&mut cmd, tools)?;
    }
    apply_platform_tools(&mut cmd, &mut cargo_args)?;
    if let Some(p) = &pool {
        cmd.env(crate::test_pool::POOL_ENV, p.sock());
    }
    let recording_rustc = resident_recording_compiler(
        std::env::var_os("JUSTRUST_CARGO_RESIDENT_DIR").as_deref(),
        pinned.as_ref().map(|tc| tc.id.as_str()),
        explicit_resident_candidates(
            std::env::var_os("JUSTRUST_TOOLCHAIN").as_deref(),
            std::env::var_os("RUSTC").as_deref(),
            std::env::var_os("JUSTRUST_REAL_CARGO").as_deref(),
        ),
        &real_rustc,
        &shim,
    )?;
    cmd.args(&cargo_args)
        .env_remove("JUSTRUST_RECORD_PHASE_TRACE")
        .env("RUSTC", recording_rustc)
        .env("JUSTRUST_REAL_RUSTC", &real_rustc)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_recording_run_env(
        &mut cmd,
        recording_rustc == real_rustc.as_path(),
        &run_dir,
        id,
    );
    unify_features(&mut cmd);
    if opts.agent && std::env::var_os("CARGO_TERM_VERBOSE").is_none() {
        // Makes cargo say why each unit was rebuilt ("Dirty foo: the file
        // `src/lib.rs` has changed"). The agent filter hides the extra lines;
        // the report uses them.
        cmd.env("CARGO_TERM_VERBOSE", "true");
    }
    if user_progress {
        if std::env::var_os("CARGO_TERM_COLOR").is_none() {
            cmd.env("CARGO_TERM_COLOR", "always");
        }
        if std::env::var_os("CARGO_TERM_PROGRESS_WHEN").is_none()
            && let Some(w) = term_width(2)
        {
            cmd.env("CARGO_TERM_PROGRESS_WHEN", "always");
            cmd.env("CARGO_TERM_PROGRESS_WIDTH", w.to_string());
        }
    }

    // Establish the CPU interval before Cargo can do any work. A baseline
    // taken by the sampler thread after spawn loses most of a short build.
    let baseline = SampleBaseline::capture();
    crate::e2e::mark("record_baseline");
    let start = baseline.sample.t;
    // Early diagnostics (opt-in): a resident rust-analyzer answers in
    // parallel with cargo. Never touches cargo's command or output.
    let build_done: Arc<std::sync::atomic::AtomicBool> = Arc::default();
    let early_rx = if opts.agent {
        std::env::current_dir().ok().and_then(|cwd| {
            crate::early::run_for_build(&meta.subcommand, cwd, start, build_done.clone())
        })
    } else {
        None
    };
    install_signal_forwarding();
    trace.mark("prepare");
    if let Some(lease) = &_compiler_lease {
        lease.inherit(&mut cmd);
    }
    drop(selection);
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning {}", real_cargo.display()))?;
    crate::e2e::mark("cargo_spawn");
    let job = crate::sys::Job::for_child(&child);
    let pid = child.id() as i32;
    lifecycle.running();
    CHILD_PID.store(pid, Ordering::SeqCst);
    // A signal can arrive while spawn is publishing the cargo PID.
    #[cfg(unix)]
    if let sig @ 1.. = crate::test_pool::cancellation_signal() {
        forward_signal(sig);
    }

    let out = child.stdout.take().context("stdout")?;
    let err = crate::live::SniffReader::new(
        child.stderr.take().context("stderr")?,
        crate::live::Sniffer::new(Some(&run_dir), strip_progress),
    );
    let t_out = spawn_tee(
        out,
        std::io::stdout(),
        "o",
        lines.clone(),
        f_out.clone(),
        out_lock.clone(),
    );
    let t_err = spawn_tee(
        err,
        std::io::stderr(),
        "e",
        lines.clone(),
        f_err.clone(),
        out_lock.clone(),
    );

    let done: Arc<(Mutex<bool>, std::sync::Condvar)> = Arc::default();
    let sampler = {
        let done = done.clone();
        let daemon_pid = run_dir.join(DAEMON_PID_FILE);
        let sample_dir = run_dir.clone();
        let lines = lines.clone();
        let log_availability = lifecycle.log_availability();
        std::thread::spawn(move || {
            sample_loop(
                pid,
                &daemon_pid,
                &sample_dir,
                &done,
                &lines,
                &log_availability,
                baseline,
            )
        })
    };

    let (mut code, mut cpu_user, cpu_sys) = wait_child(&mut child, job.as_ref());
    trace.mark("cargo");
    // Cargo may finish long before its pooled tests. Do not retain a reaped
    // PID that the OS could reuse while those tests are still running.
    CHILD_PID.store(0, Ordering::SeqCst);
    let _ = t_out.join();
    let _ = t_err.join();
    // Cargo has exited; release the slot lock before draining pooled tests so
    // an identical build waiting on this slot does not sit behind our test run.
    if let Some(s) = slot.as_mut() {
        s.release();
    }
    let mut pooled_spans = Vec::new();
    if let Some(p) = pool {
        // Binaries cargo handed over may still be running.
        let o = p.finish();
        pooled_spans = o.process_spans;
        cpu_user += o.cpu_secs;
        if o.any_failed && code == 0 {
            code = 101;
        }
    }
    let end = paths::now();
    crate::e2e::mark("cargo_done");
    {
        let (lock, cv) = &*done;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    // Do not postpone the endpoint observation behind cgroup accounting.
    // That work is outside the build interval and can be slow.
    let sched = scope.map(crate::sched::Scope::finish);
    trace.mark("drain-scope");
    build_done.store(true, Ordering::SeqCst);
    // A short grace period only: the daemon answers in about 0.1 s, and a
    // slow or stuck one must not hold up the build's exit.
    let early = early_rx.and_then(|rx| rx.recv_timeout(std::time::Duration::from_millis(50)).ok());
    let (samples, procs) = sampler.join().unwrap_or_default();
    trace.mark("sampler");
    let git = git.join().unwrap_or_default();
    trace.mark("git-join");

    let (lines, journal_complete) = {
        let mut capture = lines.lock().unwrap();
        let complete = capture.journal.flush();
        (std::mem::take(&mut capture.lines), complete)
    };
    let logs_complete = persist_output(&run_dir, &lines, journal_complete);
    if !logs_complete {
        crate::telemetry::record_persistence_failure(
            id,
            crate::telemetry::PersistenceFailure::Log,
            code,
        );
    }
    let mut hidden = agent_output::Hidden::default();
    for f in [&f_out, &f_err] {
        if let Some(f) = f.as_ref() {
            hidden.merge(f.lock().unwrap().hidden);
        }
    }
    match finish(
        opts,
        hidden,
        early,
        sched,
        &run_dir,
        &meta,
        git,
        start,
        end,
        code,
        cpu_user + cpu_sys,
        &lines,
        &samples,
        &procs,
        &pooled_spans,
    ) {
        Ok(failure) => lifecycle.set_failure(failure),
        Err(e) => {
            eprintln!("justrust: failed to write run summary: {e:#}");
            crate::telemetry::record_persistence_failure(
                id,
                crate::telemetry::PersistenceFailure::Summary,
                code,
            );
        }
    }
    lifecycle.done(code, logs_complete);
    trace.mark("finish");
    // Early diagnostics, auto mode: warm a daemon for busy workspaces,
    // in the background, after the build.
    if opts.agent {
        crate::early::maybe_autostart(Path::new(&meta.cwd), &meta.subcommand);
    }
    if let Some(s) = &slot {
        if code != 0 {
            slots::heal_after_failure(&s.info, args, lines.iter().map(|l| l.l.as_str()));
        }
        // Detached and at most hourly; never delays or fails this run.
        slots::maybe_gc_background(&s.info);
    }
    // Stale incremental state: detached, at most hourly per directory.
    let incr_target = match &slot {
        Some(s) => {
            crate::incr_gc::target_base(Path::new(&s.info.target_dir), s.info.slot.is_some())
        }
        None => std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| Path::new(&meta.cwd).join("target")),
    };
    crate::incr_gc::maybe_background(Some(&incr_target));
    trace.mark("cleanup");
    Ok(code)
}

#[derive(Debug)]
struct ToolchainRequired(String);

impl std::fmt::Display for ToolchainRequired {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ToolchainRequired {}

fn require_selected_toolchain(requested: Option<&str>, selected: Option<&str>) -> Result<()> {
    if let Some(want) = requested
        && selected != Some(want)
    {
        return Err(ToolchainRequired(format!(
            "toolchain required: {want} is unavailable, refusing system compiler fallback"
        ))
        .into());
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn finish(
    opts: Options,
    hidden: agent_output::Hidden,
    early: Option<crate::early::EarlyInfo>,
    sched: Option<crate::sched::SchedInfo>,
    run_dir: &Path,
    meta: &Meta,
    git: GitInfo,
    start: f64,
    end: f64,
    code: i32,
    cpu_secs: f64,
    lines: &[OutputLine],
    samples: &[Sample],
    procs: &[ProcRecord],
    pooled_spans: &[(f64, f64)],
) -> Result<Option<crate::failure::Wire>> {
    write_jsonl(&run_dir.join("samples.jsonl"), samples)?;
    std::fs::write(
        run_dir.join("processes.json"),
        serde_json::to_vec_pretty(procs)?,
    )?;
    let mut units = summary::load_units(run_dir);
    units.extend(summary::units_from_wrapper_procs(procs, run_dir));
    let mut s = summary::build(
        meta,
        git,
        start,
        end,
        code,
        cpu_secs,
        lines,
        samples,
        procs,
        &units,
        pooled_spans,
    );
    s.sched = sched;
    s.savings = crate::savings::from_dir(run_dir);
    s.lock_holders = std::mem::take(&mut *LOCK_HOLDERS.lock().unwrap());
    if let Some((pre_id, waited)) = crate::buildd::waited_prebuild() {
        // Its record is next to this run's: it exited before this run began.
        let pre = run_dir
            .parent()
            .and_then(|runs| std::fs::read(runs.join(&pre_id).join("summary.json")).ok())
            .and_then(|b| serde_json::from_slice::<summary::Summary>(&b).ok());
        s.credit_prebuild(&pre_id, waited, pre.as_ref());
    }
    // Waited in a slot for the identical build: that run compiled the units.
    if let Some(joined) = s.slot.as_ref().and_then(|i| i.joined_run.clone())
        && let Some(j) = run_dir
            .parent()
            .and_then(|runs| std::fs::read(runs.join(&joined).join("summary.json")).ok())
            .and_then(|b| serde_json::from_slice::<summary::Summary>(&b).ok())
    {
        s.credit_join(&j);
    }
    if let Some(mut e) = early {
        let root = crate::early::workspace_root(Path::new(&meta.cwd))
            .unwrap_or_else(|| PathBuf::from(&meta.cwd));
        let errs = crate::early::rustc_errors(lines.iter().map(|l| l.l.as_str()), &root);
        crate::early::score(&mut e, &errs);
        s.early = Some(e);
    }
    crate::split_index::attach(&mut s);
    s.e2e = Some(crate::e2e::local(
        &crate::e2e::marks(),
        crate::e2e::elapsed(),
        &s.phases,
    ));
    std::fs::write(run_dir.join("summary.json"), serde_json::to_vec_pretty(&s)?)?;
    paths::append_line(
        &paths::index_file()?,
        &serde_json::to_string(&s.index_entry())?,
    )?;
    crate::split_index::record(&s);
    crate::split_index::maybe_refresh(&s);
    crate::test_reuse::store(&s, s.e2e.as_ref().map_or(s.wall, |e| e.wall));
    crate::depcache_gc::maybe_spawn();
    crate::depcache_remote::maybe_spawn_sync();
    crate::telemetry::record_run(&s, opts.agent);
    crate::export::upload_run(&s);
    crate::telemetry::maybe_spawn_background();
    if s.diagnostics
        .failure
        .as_ref()
        .is_some_and(|f| f.class == crate::failure::Class::Toolchain)
    {
        crate::stock_verify::maybe_spawn(&s.id);
    }
    if opts.agent {
        eprint!("{}", s.agent_footer(&hidden));
        eprint!("{}", crate::hints::render(&crate::hints::for_run(&s)));
    } else if std::env::var_os("JUSTRUST_QUIET").is_none() {
        eprintln!("justrust: {}", s.one_line());
    }
    Ok(s.failure_wire())
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    #[test]
    fn failed_journal_keeps_capture_and_atomic_repair_preserves_prefix() {
        let dir = std::env::temp_dir().join(format!(
            "jr-journal-{}-{}",
            std::process::id(),
            paths::now()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let capture = Mutex::new(OutputCapture {
            lines: Vec::new(),
            journal: crate::log_journal::Journal::new(&dir),
        });
        push_line(&capture, 1.25, "o", b"\x1b[32mexact marker\x1b[0m");
        let mut capture = capture.into_inner().unwrap();
        assert!(!capture.journal.flush());
        assert_eq!(capture.lines[0].l, "exact marker");
        std::fs::write(dir.join("output.jsonl"), b"prefix\n").unwrap();
        std::fs::create_dir(dir.join("output.complete.jsonl")).unwrap();
        assert!(!persist_output(&dir, &capture.lines, false));
        assert_eq!(
            std::fs::read(dir.join("output.jsonl")).unwrap(),
            b"prefix\n"
        );
        std::fs::remove_dir(dir.join("output.complete.jsonl")).unwrap();
        assert!(persist_output(&dir, &capture.lines, false));
        let saved = std::fs::read_to_string(dir.join("output.jsonl")).unwrap();
        assert_eq!(saved.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<OutputLine>(saved.trim()).unwrap().l,
            "exact marker"
        );
        assert!(persist_output(&dir, &[], true));
        assert_eq!(
            std::fs::read_to_string(dir.join("output.jsonl")).unwrap(),
            saved
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}

fn persist_output(run_dir: &Path, lines: &[OutputLine], journal_complete: bool) -> bool {
    if journal_complete {
        return true;
    }
    // Publish repair only after a complete write, preserving the prefix on failure.
    let staging = run_dir.join("output.complete.jsonl");
    let result = write_jsonl(&staging, lines)
        .and_then(|()| Ok(std::fs::rename(&staging, run_dir.join("output.jsonl"))?));
    if let Err(error) = result {
        let _ = std::fs::remove_file(staging);
        eprintln!("justrust: failed to write output journal: {error:#}");
        return false;
    }
    true
}

fn write_jsonl<T: Serialize>(path: &Path, items: &[T]) -> Result<()> {
    let mut f = std::io::BufWriter::new(std::fs::File::create(path)?);
    for item in items {
        serde_json::to_writer(&mut f, item)?;
        f.write_all(b"\n")?;
    }
    f.flush()?;
    Ok(())
}

/// Wait for cargo. Returns (exit code, user CPU, system CPU) where CPU covers
/// cargo and every descendant it waited for (rustc, linkers, tests): from
/// `RUSAGE_CHILDREN` on Unix, from the job object holding cargo's tree on
/// Windows.
fn wait_child(child: &mut std::process::Child, job: Option<&crate::sys::Job>) -> (i32, f64, f64) {
    let u = crate::sys::wait_usage(child, job);
    #[cfg(unix)]
    let (user, sys) = crate::sys::children_cpu();
    #[cfg(windows)]
    let (user, sys) = (u.user, u.sys);
    (u.code, user, sys)
}

fn spawn_tee<R: Read + Send + 'static, W: Write + Send + 'static>(
    mut src: R,
    mut dst: W,
    stream: &'static str,
    lines: Arc<Mutex<OutputCapture>>,
    filter: Arc<Option<Mutex<agent_output::Filter>>>,
    out_lock: Arc<Mutex<()>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if let Some(filter) = filter.as_ref() {
            return tee_filtered(src, dst, stream, &lines, filter, &out_lock);
        }
        let mut buf = vec![0u8; 64 * 1024];
        let mut pending: Vec<u8> = Vec::new();
        loop {
            let n = match src.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            };
            {
                let _g = out_lock.lock();
                let _ = dst.write_all(&buf[..n]);
                let _ = dst.flush();
            }
            let t = paths::now();
            pending.extend_from_slice(&buf[..n]);
            while let Some(pos) = pending.iter().position(|&b| b == b'\n') {
                let raw: Vec<u8> = pending.drain(..=pos).collect();
                push_line(&lines, t, stream, &raw[..raw.len() - 1]);
            }
        }
        if !pending.is_empty() {
            push_line(&lines, paths::now(), stream, &pending);
        }
    })
}

/// Line-buffered tee that only forwards lines the agent filter keeps. Every
/// line is still recorded.
fn tee_filtered<R: Read, W: Write>(
    src: R,
    mut dst: W,
    stream: &str,
    lines: &Mutex<OutputCapture>,
    filter: &Mutex<agent_output::Filter>,
    out_lock: &Mutex<()>,
) {
    use std::io::BufRead;
    let mut reader = std::io::BufReader::new(src);
    let mut raw = Vec::new();
    loop {
        raw.clear();
        match reader.read_until(b'\n', &mut raw) {
            Ok(0) => break,
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        let body = raw.strip_suffix(b"\n").unwrap_or(&raw);
        let body = body.strip_suffix(b"\r").unwrap_or(body);
        let text = strip_ansi(&String::from_utf8_lossy(body));
        let _g = out_lock.lock();
        if filter.lock().unwrap().keep(&text) {
            let _ = dst.write_all(text.as_bytes());
            let _ = dst.write_all(b"\n");
            let _ = dst.flush();
        }
        push_line(lines, paths::now(), stream, body);
    }
}

/// Where the test pool prints: the same filters, saved output, and terminal
/// as cargo's own output, one binary's block at a time.
struct PoolSink {
    lines: Arc<Mutex<OutputCapture>>,
    f_out: Arc<Option<Mutex<agent_output::Filter>>>,
    f_err: Arc<Option<Mutex<agent_output::Filter>>>,
    out_lock: Arc<Mutex<()>>,
}

impl crate::test_pool::Sink for PoolSink {
    fn emit_block(&self, block: &[(f64, &'static str, Vec<u8>)]) {
        let _g = self.out_lock.lock();
        let (mut out, mut err) = (std::io::stdout().lock(), std::io::stderr().lock());
        for (t, stream, raw) in block {
            let to_stdout = *stream == "o";
            let filter = if to_stdout { &self.f_out } else { &self.f_err };
            let dst: &mut dyn Write = if to_stdout { &mut out } else { &mut err };
            match filter.as_ref() {
                Some(f) => {
                    let text = strip_ansi(&String::from_utf8_lossy(raw));
                    if f.lock().unwrap().keep(&text) {
                        let _ = dst.write_all(text.as_bytes());
                        let _ = dst.write_all(b"\n");
                    }
                }
                None => {
                    let _ = dst.write_all(raw);
                    let _ = dst.write_all(b"\n");
                }
            }
            push_line(&self.lines, *t, stream, raw);
        }
        let _ = out.flush();
        let _ = err.flush();
    }
}

fn push_line(lines: &Mutex<OutputCapture>, t: f64, stream: &str, raw: &[u8]) {
    // Progress bars rewrite the line with '\r'. Keep only the final segment.
    let raw = raw
        .rsplit(|&b| b == b'\r')
        .find(|seg| !seg.is_empty())
        .unwrap_or(&[]);
    let text = strip_ansi(&String::from_utf8_lossy(raw));
    if text.trim().is_empty() {
        return;
    }
    if text.contains("Blocking waiting for file lock on ") {
        note_lock_holders();
    }
    let line = OutputLine {
        t,
        s: stream.to_owned(),
        l: text,
    };
    let mut capture = lines.lock().unwrap();
    capture.journal.append(&line);
    capture.lines.push(line);
}

/// Who held the locks this run waited for ([`procfs::lock_holders_blocking_us`]).
static LOCK_HOLDERS: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Cargo said it is blocked on a file lock: look at who holds it, off the
/// output thread. Cargo prints before it blocks, and the holder can change
/// during a long wait, so look twice.
fn note_lock_holders() {
    std::thread::spawn(|| {
        for wait_ms in [100, 2000] {
            std::thread::sleep(Duration::from_millis(wait_ms));
            let found = procfs::lock_holders_blocking_us();
            let mut all = LOCK_HOLDERS.lock().unwrap();
            for h in found {
                if !all.contains(&h) && all.len() < 8 {
                    all.push(h);
                }
            }
        }
    });
}

pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for c in chars.by_ref() {
                    if ('@'..='~').contains(&c) {
                        break;
                    }
                }
            } else {
                chars.next();
            }
            continue;
        }
        out.push(c);
    }
    out
}

/// `args` with cargo's progress bar forced on, placed after a `+toolchain`.
fn with_progress_config(args: &[OsString]) -> Vec<OsString> {
    let args = insert_config(args, "term.progress.width=200");
    insert_config(&args, "term.progress.when=\"always\"")
}

/// `args` with `--config <c>` placed first, after a `+toolchain`.
fn insert_config(args: &[OsString], c: &str) -> Vec<OsString> {
    let at = usize::from(
        args.first()
            .is_some_and(|a| a.to_string_lossy().starts_with('+')),
    );
    let mut out = args[..at].to_vec();
    out.push("--config".into());
    out.push(c.into());
    out.extend_from_slice(&args[at..]);
    out
}

fn classify_proc(comm: &str, cmdline: &str) -> &'static str {
    let exe = cmdline.split_whitespace().next().unwrap_or("");
    match comm {
        "cargo" => return "cargo",
        "rustc" | "clippy-driver" | "rustdoc" => return "rustc",
        "cc" | "c++" | "gcc" | "g++" | "clang" | "clang++" | "ld" | "ld.lld" | "ld.bfd"
        | "ld.gold" | "mold" | "wild" | "collect2" | "lld" | "fast-linker" => return "linker",
        _ => {}
    }
    if crate::gaps::is_build_script(comm, exe) {
        return "build-script";
    }
    if crate::sys::has_dir(exe, "target") || crate::sys::has_dir(exe, "deps") {
        return "target-binary";
    }
    if crate::sys::program_name(exe.as_ref()) == "rustc" && exe.contains(['/', '\\'])
        || cmdline.contains("rustc-parallel-frontend")
    {
        return "rustc";
    }
    "other"
}

#[derive(Default)]
struct Tracked {
    rec: Option<ProcRecord>,
    start_ticks: u64,
    first_cpu_ticks: u64,
    last_cpu_ticks: u64,
}

fn sampled_tree_cpu_delta<'a>(
    tree: impl IntoIterator<Item = &'a procfs::ProcStat>,
    tracked: &mut HashMap<(i32, u64), Tracked>,
) -> u64 {
    // Keep each process identity's last observation after it exits. Subtracting
    // sums of only live processes loses survivor work whenever a child exits.
    // Repeated roots and regressing reads must not count lifetime CPU twice.
    let mut delta = 0u64;
    for p in tree {
        let previous = &mut tracked
            .entry((p.pid, p.start_ticks))
            .or_default()
            .last_cpu_ticks;
        delta = delta.saturating_add(p.cpu_ticks.saturating_sub(*previous));
        *previous = (*previous).max(p.cpu_ticks);
    }
    delta
}

/// A jr-cargo daemon child (default on, `JUSTRUST_CARGO_DAEMON=0` opts out) runs the build
/// outside cargo's process tree and writes its pid here.
const DAEMON_PID_FILE: &str = "cargo-daemon.pid";

struct SampleBaseline {
    sample: Sample,
    cpu: Option<procfs::CpuTimes>,
    psi: Option<procfs::Pressure>,
    tracked: HashMap<(i32, u64), Tracked>,
}

impl SampleBaseline {
    fn capture() -> Self {
        let mem_available_mb = procfs::meminfo().map(|m| m.available_mb).unwrap_or(0);
        let processes: Vec<_> = procfs::all_pids()
            .into_iter()
            .filter_map(procfs::read_stat)
            .collect();
        let foreign_rustc_procs = processes.iter().filter(|p| p.comm == "rustc").count();
        // Existing daemons can join the build tree later. Their pre-build
        // lifetime CPU is not work done during this run.
        let mut tracked = HashMap::new();
        sampled_tree_cpu_delta(&processes, &mut tracked);
        let psi = procfs::pressure();
        let cpu = procfs::cpu_times();
        Self {
            sample: Sample {
                t: paths::now(),
                sys_cores: 0.0,
                build_cores: 0.0,
                build_rss_mb: 0.0,
                mem_available_mb,
                rustc_procs: 0,
                foreign_rustc_procs,
                psi_cpu_ms: 0.0,
                psi_mem_ms: 0.0,
                psi_io_ms: 0.0,
                psi_unavailable: psi.is_none(),
            },
            cpu,
            psi,
            tracked,
        }
    }
}

fn sample_loop(
    root: i32,
    daemon_pid: &Path,
    run_dir: &Path,
    done: &(Mutex<bool>, std::sync::Condvar),
    lines: &Mutex<OutputCapture>,
    log_availability: &std::sync::atomic::AtomicBool,
    baseline: SampleBaseline,
) -> (Vec<Sample>, Vec<ProcRecord>) {
    let sample_start = std::time::Instant::now();
    let mut last_log_flush = sample_start;
    let mut daemon: Option<i32> = None;
    let hz = procfs::clk_tck();
    let page_mb = procfs::page_size() as f64 / 1_048_576.0;
    let ncpu = procfs::ncpu() as f64;
    let mut prev_cpu = baseline.cpu;
    let mut prev_psi = baseline.psi;
    let mut prev_t = baseline.sample.t;
    let mut samples = vec![baseline.sample];
    let mut tracked = baseline.tracked;
    loop {
        let stop = *done.0.lock().unwrap();
        let t = paths::now();
        let all: Vec<procfs::ProcStat> = procfs::all_pids()
            .into_iter()
            .filter_map(procfs::read_stat)
            .collect();
        if daemon.is_none() {
            daemon = std::fs::read_to_string(daemon_pid)
                .ok()
                .and_then(|s| s.trim().parse().ok());
        }
        // Pool children and the daemon may overlap cargo's process tree.
        let roots = std::iter::once(root)
            .chain(daemon)
            .chain(crate::test_pool::live_pids());
        let (tree, in_tree) = process_forest(roots, &all);
        let tree_delta_ticks = sampled_tree_cpu_delta(tree.iter().copied(), &mut tracked);
        let mut rss = 0u64;
        let mut rustc_procs = 0;
        for p in &tree {
            let entry = tracked.entry((p.pid, p.start_ticks)).or_default();
            if entry.rec.is_none() {
                let cmdline = procfs::cmdline(p.pid);
                let kind = classify_proc(&p.comm, &cmdline).to_owned();
                entry.start_ticks = p.start_ticks;
                entry.first_cpu_ticks = p.cpu_ticks;
                entry.rec = Some(ProcRecord {
                    pid: p.pid,
                    comm: p.comm.clone(),
                    cmdline: cmdline.chars().take(600).collect(),
                    kind,
                    first_seen: t,
                    last_seen: t,
                    cpu_secs: 0.0,
                    max_rss_mb: 0.0,
                });
            }
            let rec = entry.rec.as_mut().unwrap();
            rec.last_seen = t;
            rec.cpu_secs = p.cpu_ticks as f64 / hz;
            rec.max_rss_mb = rec.max_rss_mb.max(p.rss_pages as f64 * page_mb);
            if rec.kind == "rustc" {
                rustc_procs += 1;
            }
            rss += p.rss_pages;
        }
        let foreign = all
            .iter()
            .filter(|p| p.comm == "rustc" && !in_tree.contains(&p.pid))
            .count();

        let cpu = procfs::cpu_times();
        let psi = procfs::pressure();
        // Timestamp the counter observation, not the start of the process scan.
        let t = paths::now();
        let stall = |f: fn(&procfs::Pressure) -> u64| match (&psi, &prev_psi) {
            (Some(b), Some(a)) => f(b).saturating_sub(f(a)) as f64 / 1000.0,
            _ => 0.0,
        };
        let dt = (t - prev_t).max(1e-3);
        let sys_cores = match (prev_cpu, cpu) {
            (Some(a), Some(b)) if b.total > a.total => {
                (b.busy - a.busy) as f64 / (b.total - a.total) as f64 * ncpu
            }
            _ => 0.0,
        };
        let build_cores = tree_delta_ticks as f64 / hz / dt;
        write_live_cpu(
            run_dir,
            &LiveCpu {
                t,
                build_cores: build_cores.min(ncpu),
                ncpu: ncpu as usize,
            },
        );
        samples.push(Sample {
            t,
            sys_cores,
            build_cores: build_cores.min(ncpu),
            build_rss_mb: rss as f64 * page_mb,
            mem_available_mb: procfs::meminfo().map(|m| m.available_mb).unwrap_or(0),
            rustc_procs,
            foreign_rustc_procs: foreign,
            psi_cpu_ms: stall(|p| p.cpu_us),
            psi_mem_ms: stall(|p| p.memory_us),
            psi_io_ms: stall(|p| p.io_us),
            psi_unavailable: psi.is_none(),
        });
        prev_cpu = cpu;
        prev_psi = psi;
        prev_t = t;
        // Persist output after observing counters. In particular, a contended
        // output lock or slow journal flush must not delay the final endpoint.
        if stop || last_log_flush.elapsed() >= Duration::from_secs(1) {
            let persisted = lines.lock().unwrap().journal.flush();
            log_availability.store(persisted, Ordering::Relaxed);
            last_log_flush = std::time::Instant::now();
        }
        if stop {
            break;
        }
        // Resolve short warm builds with real observations, then return to the
        // normal cadence. Use monotonic elapsed time only for scheduling, never
        // for sample timestamps. Waiting after collection avoids catch-up scans
        // if collecting a sample itself is slow. Completion still wakes us
        // immediately, without waiting for a minimum number of samples.
        let interval = sample_interval(sample_start.elapsed());
        let guard = done.0.lock().unwrap();
        let _ = done
            .1
            .wait_timeout_while(guard, interval, |finished| !*finished);
    }
    let mut procs: Vec<ProcRecord> = tracked
        .into_values()
        .filter_map(|mut t| {
            let mut r = t.rec.take()?;
            // CPU used while observed (processes already running when first seen
            // still count their earlier CPU, which is what we want for children).
            let _ = t.first_cpu_ticks;
            r.cpu_secs = t.last_cpu_ticks as f64 / hz;
            Some(r)
        })
        .collect();
    procs.sort_by(|a, b| a.first_seen.total_cmp(&b.first_seen));
    (samples, procs)
}

fn process_forest(
    roots: impl IntoIterator<Item = i32>,
    all: &[procfs::ProcStat],
) -> (Vec<&procfs::ProcStat>, std::collections::HashSet<i32>) {
    let mut children: HashMap<i32, Vec<&procfs::ProcStat>> = HashMap::new();
    let mut by_pid = HashMap::with_capacity(all.len());
    for p in all {
        children.entry(p.ppid).or_default().push(p);
        // Use the first observation for root output and the last start time
        // for creator validation, even if enumeration repeated a PID.
        by_pid.entry(p.pid).or_insert((p, p.start_ticks)).1 = p.start_ticks;
    }
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut stack = Vec::new();
    for root in roots {
        if !seen.insert(root) {
            continue;
        }
        let observed = by_pid.get(&root);
        if let Some(&(p, _)) = observed {
            out.push(p);
        }
        // An exited root can still have live children in this snapshot.
        stack.push((root, observed.map(|p| p.1)));
        while let Some((pid, start)) = stack.pop() {
            for &child in children.get(&pid).into_iter().flatten() {
                // Windows retains historical creator PIDs. Reject reused
                // parents before claiming membership for this root.
                if start.is_some_and(|s| s > child.start_ticks) || !seen.insert(child.pid) {
                    continue;
                }
                out.push(child);
                stack.push((child.pid, Some(by_pid[&child.pid].1)));
            }
        }
    }
    (out, seen)
}

fn git_info() -> GitInfo {
    let run = |args: &[&str]| -> Option<String> {
        let o = Command::new("git")
            .args(args)
            .stderr(Stdio::null())
            .output()
            .ok()?;
        o.status
            .success()
            .then(|| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let (root, head) = git_root_head(run);
    GitInfo {
        root,
        head,
        dirty_files: run(&["status", "--porcelain", "--untracked-files=no"])
            .map(|s| s.lines().count()),
        dirty_rs_lines: run(&["diff", "--numstat", "HEAD", "--", "*.rs"]).map(parse_numstat_lines),
    }
}

pub(crate) fn dirty_rs_lines_in(dir: &Path) -> Option<usize> {
    let o = Command::new("git")
        .current_dir(dir)
        .args(["diff", "--numstat", "HEAD", "--", "*.rs"])
        .stderr(Stdio::null())
        .output()
        .ok()?;
    o.status
        .success()
        .then(|| parse_numstat_lines(String::from_utf8_lossy(&o.stdout).trim().to_owned()))
}

fn parse_numstat_lines(out: String) -> usize {
    out.lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let add: usize = parts.next()?.parse().ok()?;
            let del: usize = parts.next()?.parse().ok()?;
            Some(add.saturating_add(del))
        })
        .sum()
}

fn git_root_head(run: impl Fn(&[&str]) -> Option<String>) -> (Option<String>, Option<String>) {
    if let Some(both) = run(&["rev-parse", "--show-toplevel", "--short=12", "HEAD"])
        && let Some((root, head)) = both.rsplit_once('\n')
    {
        return (Some(root.trim().to_owned()), Some(head.trim().to_owned()));
    }
    // An unborn repository still has a root. A bare repository can have a
    // valid HEAD without a working tree. Keep those independent observations.
    (
        run(&["rev-parse", "--show-toplevel"]),
        run(&["rev-parse", "--short=12", "HEAD"]),
    )
}

/// Opt-in workspace-wide feature resolution can enable defaults of dependencies
/// in other workspaces that the selected package explicitly disabled. Preserve
/// Cargo's selected-package semantics unless requested, and respect explicit
/// CARGO_RESOLVER_FEATURE_UNIFICATION configuration.
fn workspace_features_requested(request: Option<&str>, explicit_resolver: bool) -> bool {
    !explicit_resolver && matches!(request, Some("1" | "on" | "true" | "yes"))
}

fn unify_features(cmd: &mut Command) {
    if !workspace_features_requested(
        std::env::var("JUSTRUST_UNIFY_FEATURES").ok().as_deref(),
        std::env::var_os("CARGO_RESOLVER_FEATURE_UNIFICATION").is_some(),
    ) {
        return;
    }
    cmd.env("CARGO_RESOLVER_FEATURE_UNIFICATION", "workspace")
        .env("CARGO_UNSTABLE_FEATURE_UNIFICATION", "true");
    if std::env::var_os("RUSTC_BOOTSTRAP").is_none() {
        cmd.env("__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS", "nightly");
    }
}

/// Per-process shim destinations are not compiler inputs in the explicit native
/// experiment. Omit them at the Cargo boundary in BOTH its off/on controls,
/// rather than silently stripping observable environment inside the worker.
fn apply_recording_run_env(cmd: &mut Command, native: bool, run_dir: &Path, id: &str) {
    if native {
        cmd.env_remove("JUSTRUST_RUN_DIR")
            .env_remove("JUSTRUST_RUN_ID");
    } else {
        cmd.env("JUSTRUST_RUN_DIR", run_dir)
            .env("JUSTRUST_RUN_ID", id);
    }
}

/// The resident experiment must enter the owned compiler directly. Its native
/// Cargo path owns transport and lifetime accounting, so the one-process shim
/// must not add per-invocation phase files or wrap the resident worker. This is
/// deliberately opt-in and does not change stock-toolchain recording.
/// Explicit component candidates need not masquerade as a fully installed
/// toolchain. Cargo still probes the compiler's native capability before RPC.
/// Requiring the system selection and both absolute overrides keeps this path
/// separate from published pins and from accidental PATH compiler selection.
fn explicit_resident_candidates(
    selection: Option<&std::ffi::OsStr>,
    compiler: Option<&std::ffi::OsStr>,
    cargo: Option<&std::ffi::OsStr>,
) -> bool {
    selection == Some(std::ffi::OsStr::new("system"))
        && compiler.is_some_and(|p| !p.is_empty() && Path::new(p).is_absolute())
        && cargo.is_some_and(|p| !p.is_empty() && Path::new(p).is_absolute())
}

fn resident_recording_compiler<'a>(
    directory: Option<&std::ffi::OsStr>,
    toolchain: Option<&str>,
    explicit_candidates: bool,
    compiler: &'a Path,
    shim: &'a Path,
) -> Result<&'a Path> {
    let Some(directory) = directory else {
        return Ok(shim);
    };
    anyhow::ensure!(
        !directory.is_empty() && Path::new(directory).is_absolute(),
        "JUSTRUST_CARGO_RESIDENT_DIR requires an absolute private guest-side directory"
    );
    anyhow::ensure!(
        toolchain.is_some_and(|id| id.starts_with("jr-"))
            || (toolchain.is_none() && explicit_candidates),
        "the native Cargo resident experiment requires an owned jr toolchain or JUSTRUST_TOOLCHAIN=system with absolute RUSTC and JUSTRUST_REAL_CARGO candidates"
    );
    Ok(compiler)
}

#[cfg(test)]
mod tests {
    #[test]
    fn combined_git_metadata_retains_newline_paths_and_partial_results() {
        assert_eq!(
            super::git_root_head(|args| {
                assert_eq!(args.len(), 4);
                Some("/a\nb\n0123456789ab".into())
            }),
            (Some("/a\nb".into()), Some("0123456789ab".into()))
        );
        for (root, head) in [
            (Some("/empty"), None),
            (None, Some("0123456789ab")),
            (None, None),
        ] {
            assert_eq!(
                super::git_root_head(|args| match args {
                    ["rev-parse", "--show-toplevel"] => root.map(str::to_owned),
                    ["rev-parse", "--short=12", "HEAD"] => head.map(str::to_owned),
                    _ => None,
                }),
                (root.map(str::to_owned), head.map(str::to_owned))
            );
        }
    }
    #[test]
    fn native_recording_omits_only_unused_shim_destinations_at_cargo_boundary() {
        use std::ffi::OsStr;
        use std::path::Path;
        for native in [false, true] {
            let mut cmd = std::process::Command::new("cargo");
            cmd.env("USER_BUILD_INPUT", "preserved");
            super::apply_recording_run_env(&mut cmd, native, Path::new("/run/123"), "123");
            let env = cmd.get_envs().collect::<std::collections::BTreeMap<_, _>>();
            assert_eq!(
                env[OsStr::new("USER_BUILD_INPUT")],
                Some(OsStr::new("preserved"))
            );
            assert_eq!(
                env[OsStr::new("JUSTRUST_RUN_DIR")],
                (!native).then_some(OsStr::new("/run/123"))
            );
            assert_eq!(
                env[OsStr::new("JUSTRUST_RUN_ID")],
                (!native).then_some(OsStr::new("123"))
            );
        }
    }

    #[test]
    fn resident_recording_is_explicit_and_owned_only() {
        use std::ffi::OsStr;
        use std::path::Path;

        let private = std::env::temp_dir().join("private");
        let compiler = Path::new("/candidate/bin/rustc");
        let shim = Path::new("/production/rustc-shim");
        let select = |directory, toolchain| {
            super::resident_recording_compiler(directory, toolchain, false, compiler, shim)
        };
        assert_eq!(select(None, None).unwrap(), shim);
        assert_eq!(select(None, Some("jr-release")).unwrap(), shim);
        assert!(select(Some(OsStr::new("")), Some("jr-candidate")).is_err());
        assert!(select(Some(OsStr::new("relative")), Some("jr-candidate")).is_err());
        assert!(select(Some(private.as_os_str()), None).is_err());
        assert!(select(Some(private.as_os_str()), Some("nightly-2026-10-08")).is_err());
        assert_eq!(
            select(Some(private.as_os_str()), Some("jr-candidate")).unwrap(),
            compiler
        );
    }
    #[test]
    fn explicit_resident_components_do_not_impersonate_an_installed_pin() {
        use std::ffi::OsStr;
        use std::path::Path;
        let private = std::env::temp_dir().join("private");
        let rustc = private.join("rustc");
        let cargo = private.join("cargo");
        let pair = |selection: Option<&str>, compiler: Option<&str>, cargo: Option<&str>| {
            super::explicit_resident_candidates(
                selection.map(OsStr::new),
                compiler.map(OsStr::new),
                cargo.map(OsStr::new),
            )
        };
        assert!(pair(Some("system"), rustc.to_str(), cargo.to_str()));
        assert!(!pair(None, rustc.to_str(), cargo.to_str()));
        assert!(!pair(Some("jr-release"), rustc.to_str(), cargo.to_str()));
        assert!(!pair(Some("system"), None, cargo.to_str()));
        assert!(!pair(Some("system"), rustc.to_str(), None));
        assert!(!pair(Some("system"), Some("rustc"), cargo.to_str()));
        assert!(!pair(Some("system"), rustc.to_str(), Some("cargo")));
        let compiler = Path::new("/candidate/rustc");
        let shim = Path::new("/installed/shim");
        assert_eq!(
            super::resident_recording_compiler(
                Some(private.as_os_str()),
                None,
                true,
                compiler,
                shim,
            )
            .unwrap(),
            compiler
        );
        assert!(
            super::resident_recording_compiler(
                Some(private.as_os_str()),
                Some("nightly"),
                true,
                compiler,
                shim,
            )
            .is_err()
        );
        assert_eq!(
            super::resident_recording_compiler(None, None, true, compiler, shim,).unwrap(),
            shim
        );
    }

    #[test]
    fn workspace_feature_unification_requires_explicit_opt_in() {
        // A no-default-features path dependency must not acquire its parent
        // workspace's desktop defaults in an ordinary justrust build.
        assert!(!super::workspace_features_requested(None, false));
        for value in ["0", "off", "false", "no", "", "workspace"] {
            assert!(!super::workspace_features_requested(Some(value), false));
        }
        for value in ["1", "on", "true", "yes"] {
            assert!(super::workspace_features_requested(Some(value), false));
            assert!(!super::workspace_features_requested(Some(value), true));
        }
    }

    #[test]
    fn sampling_cadence_switches_at_initial_window_boundary() {
        use std::time::Duration;

        for elapsed in [Duration::ZERO, Duration::from_micros(399_999)] {
            assert_eq!(super::sample_interval(elapsed), Duration::from_millis(50));
        }
        for elapsed in [
            Duration::from_millis(400),
            Duration::from_millis(401),
            Duration::from_secs(60),
            Duration::MAX,
        ] {
            assert_eq!(super::sample_interval(elapsed), Duration::from_millis(200));
        }
    }

    #[test]
    fn sampling_cadence_has_bounded_fast_start_without_catch_up() {
        use std::time::Duration;

        // Model waits with zero collection cost, not synthetic sample rows.
        let mut elapsed = Duration::ZERO;
        let mut deadlines = Vec::new();
        for _ in 0..10 {
            elapsed += super::sample_interval(elapsed);
            deadlines.push(elapsed.as_millis());
        }
        assert_eq!(deadlines, [50, 100, 150, 200, 250, 300, 350, 400, 600, 800]);

        // A slow collection or scheduler stall consumes the fast-start window.
        // The next wait is still a full interval, never a catch-up burst.
        let after_slow_collection = Duration::from_millis(475);
        assert_eq!(
            after_slow_collection + super::sample_interval(after_slow_collection),
            Duration::from_millis(675)
        );
    }

    #[test]
    fn short_sampling_keeps_pre_spawn_baseline_and_wakes_for_endpoint() {
        let dir =
            std::env::temp_dir().join(format!("justrust-short-sample-{}", super::new_run_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let baseline = super::SampleBaseline::capture();
        let start = baseline.sample.t;
        let lines = Mutex::new(super::OutputCapture {
            lines: Vec::new(),
            journal: crate::log_journal::Journal::new(&dir.join("output.jsonl")),
        });
        let done = (Mutex::new(false), std::sync::Condvar::new());
        let available = std::sync::atomic::AtomicBool::new(false);
        let (samples, stop) = std::thread::scope(|scope| {
            let sampler = scope.spawn(|| {
                super::sample_loop(
                    std::process::id() as i32,
                    &dir.join(super::DAEMON_PID_FILE),
                    &dir,
                    &done,
                    &lines,
                    &available,
                    baseline,
                )
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            // Stop as soon as the first genuine interval has been observed,
            // before the normal 50 ms sampling wait has to expire.
            while !dir.join("cpu.json").exists() {
                if std::time::Instant::now() >= deadline {
                    *done.0.lock().unwrap() = true;
                    done.1.notify_all();
                    panic!("sampler did not start");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            let stop = crate::paths::now();
            *done.0.lock().unwrap() = true;
            done.1.notify_all();
            (sampler.join().unwrap().0, stop)
        });
        assert!(samples.len() >= 3);
        assert_eq!(samples[0].t, start);
        assert_eq!(samples[0].sys_cores, 0.0);
        assert_eq!(samples[0].build_cores, 0.0);
        assert!(samples[1].t <= stop);
        assert!(samples.last().unwrap().t >= stop);
        assert!(samples.windows(2).all(|pair| pair[1].t > pair[0].t));
        assert!(samples.iter().all(|sample| {
            sample.sys_cores.is_finite()
                && sample.build_cores.is_finite()
                && sample.sys_cores >= 0.0
                && sample.build_cores >= 0.0
                && sample.build_cores <= crate::procfs::ncpu() as f64
        }));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn already_finished_sampling_does_not_pad_short_series() {
        let dir =
            std::env::temp_dir().join(format!("justrust-finished-sample-{}", super::new_run_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let baseline = super::SampleBaseline::capture();
        let start = baseline.sample.t;
        let stop = crate::paths::now();
        let lines = Mutex::new(super::OutputCapture {
            lines: Vec::new(),
            journal: crate::log_journal::Journal::new(&dir.join("output.jsonl")),
        });
        let samples = std::thread::scope(|scope| {
            let output_guard = lines.lock().unwrap();
            let sampler = scope.spawn(|| {
                super::sample_loop(
                    std::process::id() as i32,
                    &dir.join(super::DAEMON_PID_FILE),
                    &dir,
                    &(Mutex::new(true), std::sync::Condvar::new()),
                    &lines,
                    &std::sync::atomic::AtomicBool::new(false),
                    baseline,
                )
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            // The final CPU observation must precede the journal flush, even
            // when the output lock is held by another thread.
            while !dir.join("cpu.json").exists() {
                if std::time::Instant::now() >= deadline {
                    drop(output_guard);
                    panic!("output journal lock delayed the CPU endpoint");
                }
                std::thread::sleep(Duration::from_millis(1));
            }
            drop(output_guard);
            sampler.join().unwrap().0
        });
        // An unobservably short build must still fail strict provenance gates,
        // not gain a duplicated/interpolated row or wait for an artificial one.
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].t, start);
        assert!(samples[1].t >= stop);
        assert!(samples[1].t > samples[0].t);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sampling_baseline_excludes_existing_daemon_lifetime_cpu() {
        let mut tracked = std::collections::HashMap::new();
        super::sampled_tree_cpu_delta(&[cpu_stat(1, 1, 500)], &mut tracked);
        // An existing daemon joins alongside freshly spawned cargo. Only its
        // interval work plus the new child's work belong to the first row.
        assert_eq!(
            super::sampled_tree_cpu_delta(&[cpu_stat(1, 1, 503), cpu_stat(2, 2, 4)], &mut tracked,),
            7
        );
    }

    #[test]
    fn native_musl_uses_pinned_lld_and_requires_it() {
        let dir =
            std::env::temp_dir().join(format!("justrust-musl-linker-{}", super::new_run_id()));
        let tc = crate::toolchain::Toolchain {
            id: "1.99.0".into(),
            dir: dir.clone(),
            nightly: false,
        };
        let linker = dir
            .join("lib/rustlib")
            .join(crate::toolchain::TARGET)
            .join("bin/rust-lld");
        let args: Vec<std::ffi::OsString> = [
            "build",
            "--release",
            "--target",
            "aarch64-unknown-linux-musl",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let mut cmd = std::process::Command::new("cargo");
        assert!(super::apply_musl_linker(&mut cmd, None, &args).is_err());
        assert!(super::apply_musl_linker(&mut cmd, Some(&tc), &args).is_err());
        std::fs::create_dir_all(linker.parent().unwrap()).unwrap();
        std::fs::write(&linker, b"test linker").unwrap();
        super::apply_musl_linker_with_tools(&mut cmd, Some(&tc), &args, || {
            Ok(vec![dir.join("bin"), dir.join("zig")])
        })
        .unwrap();
        assert!(cmd.get_envs().any(|(key, value)| key
            == "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER"
            && value == Some(linker.as_os_str())));
        assert!(
            cmd.get_envs().any(|(key, value)| key == "JUSTRUST_LINKER"
                && value == Some(std::ffi::OsStr::new("off")))
        );
        assert!(
            cmd.get_envs()
                .any(|(key, value)| key == "CC_aarch64_unknown_linux_musl" && value.is_some())
        );
        let mut native = std::process::Command::new("cargo");
        super::apply_musl_linker(&mut native, None, &["build".into(), "--release".into()]).unwrap();
        assert_eq!(native.get_envs().count(), 0);
        std::fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn cross_target_arguments_override_environment_and_stop_at_separator() {
        let args = [
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "--target=aarch64-unknown-linux-gnu",
            "--",
            "--target=ignored",
        ]
        .map(OsString::from);
        assert_eq!(
            super::requested_targets(&args, Some("ignored-env".into())),
            vec![
                OsString::from("wasm32-unknown-unknown"),
                OsString::from("aarch64-unknown-linux-gnu")
            ]
        );
        assert_eq!(
            super::requested_targets(&["build".into()], Some("wasm32-unknown-unknown".into())),
            vec![OsString::from("wasm32-unknown-unknown")]
        );
        assert!(super::requested_targets(&["build".into()], None).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn target_core_probe_requires_existing_library_not_just_printed_path() {
        use std::os::unix::fs::PermissionsExt;
        let temp =
            std::env::temp_dir().join(format!("justrust-target-probe-{}", super::new_run_id()));
        let lib = temp.join("lib");
        std::fs::create_dir_all(&lib).unwrap();
        let rustc = temp.join("rustc");
        std::fs::write(
            &rustc,
            format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", lib.display()),
        )
        .unwrap();
        std::fs::set_permissions(&rustc, std::fs::Permissions::from_mode(0o755)).unwrap();
        let target = OsString::from("wasm32-unknown-unknown");
        assert!(!super::target_has_core(&rustc, &target));
        std::fs::write(lib.join("libcore-abc.rmeta"), b"").unwrap();
        // The probe reports a script it cannot run as "no". The script was
        // just written, and a child another test is forking can hold it open
        // for writing until it execs, so wait that window out.
        assert!(crate::test_fork::eventually(|| super::target_has_core(
            &rustc, &target
        )));
        std::fs::write(
            &rustc,
            format!(
                "#!/bin/sh\n[ \"$4\" = x86_64-unknown-linux-gnu ] || exit 1\nprintf '%s\\n' '{}'\n",
                lib.display()
            ),
        )
        .unwrap();
        assert!(crate::test_fork::eventually(|| super::target_has_core(
            &rustc,
            &OsString::from(crate::remote_artifacts::GLIBC_217_TARGET)
        )));
        assert!(!super::target_has_core(
            &rustc,
            &OsString::from("x86_64-unknown-linux-gnu.2.18")
        ));
        std::fs::write(&rustc, "#!/bin/sh\nexit 1\n").unwrap();
        assert!(!super::target_has_core(&rustc, &target));
        std::fs::remove_dir_all(temp).unwrap();
    }
    #[test]
    fn build_failure_telemetry_excludes_private_arguments() {
        let args = [
            "test",
            "-p",
            "private-crate",
            "--features=secret",
            "--",
            "private_test",
        ]
        .map(std::ffi::OsString::from);
        let data = super::build_failure_data(&args, "recording_init", true, None);
        assert_eq!(data["sub"], "test");
        assert_eq!(data["stage"], "recording_init");
        assert_eq!(data["fallback"], true);
        assert!(data["exit"].is_null());
        let text = data.to_string();
        assert!(!text.contains("private"));
        assert!(!text.contains("secret"));
        let unknown = [std::ffi::OsString::from("private-subcommand")];
        assert_eq!(
            super::build_failure_data(&unknown, "cargo_exec", false, Some(127))["sub"],
            "other"
        );
    }
    use super::*;

    fn cpu_stat(pid: i32, start_ticks: u64, cpu_ticks: u64) -> procfs::ProcStat {
        procfs::ProcStat {
            pid,
            ppid: 1,
            comm: "rustc".into(),
            cpu_ticks,
            start_ticks,
            rss_pages: 0,
        }
    }

    fn tree_stat(pid: i32, ppid: i32, start: u64) -> procfs::ProcStat {
        procfs::ProcStat {
            ppid,
            ..cpu_stat(pid, start, 0)
        }
    }

    #[test]
    fn process_tree_cycles_and_duplicates_are_bounded() {
        for all in [
            vec![tree_stat(1, 1, 10)],
            vec![tree_stat(1, 2, 10), tree_stat(2, 1, 10)],
            vec![
                tree_stat(1, 3, 10),
                tree_stat(2, 1, 10),
                tree_stat(3, 2, 10),
            ],
            vec![
                tree_stat(1, 0, 10),
                tree_stat(2, 1, 11),
                tree_stat(2, 1, 11),
            ],
        ] {
            let tree = process_forest([1], &all).0;
            let pids: std::collections::HashSet<_> = tree.iter().map(|p| p.pid).collect();
            assert_eq!(tree.len(), pids.len());
            assert_eq!(
                tree.len(),
                all.iter()
                    .map(|p| p.pid)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
            );
        }
        // Exact x64 OOM signature of the old unbounded Vec<ProcStat>.
        #[cfg(target_pointer_width = "64")]
        assert_eq!(
            std::mem::size_of::<procfs::ProcStat>() as u64 * (1 << 29),
            30_064_771_072
        );
    }

    #[test]
    fn process_tree_rejects_reused_parent_and_preserves_valid_descendants() {
        let all = [
            tree_stat(1, 0, 20),
            tree_stat(2, 1, 10),
            tree_stat(3, 2, 11),
            tree_stat(4, 1, 20),
            tree_stat(5, 4, 21),
            tree_stat(6, 0, 1),
        ];
        assert_eq!(
            process_forest([1], &all)
                .0
                .iter()
                .map(|p| p.pid)
                .collect::<Vec<_>>(),
            [1, 4, 5]
        );
        assert!(process_forest([99], &all).0.is_empty());
        // A root may exit between enumeration and reading process stats.
        assert_eq!(process_forest([1], &[tree_stat(2, 1, 10)]).0.len(), 1);
    }

    #[test]
    fn forest_matches_first_seen_union_of_independent_walks() {
        let all = [
            tree_stat(2, 1, 20),
            tree_stat(3, 2, 21),
            tree_stat(4, 2, 21),
            tree_stat(5, 3, 22),
            tree_stat(6, 2, 10),
            tree_stat(7, 6, 11),
            tree_stat(8, 9, 12),
            tree_stat(9, 8, 12),
            tree_stat(10, 0, 1),
        ];
        // Independent per-root reference, scanning rows rather than using the index.
        for a in 1..=10 {
            for b in 1..=10 {
                let roots = [a, b, a];
                let mut expected = Vec::new();
                for root in roots {
                    let mut walk = vec![root];
                    let mut visited = std::collections::HashSet::from([root]);
                    if all.iter().any(|s| s.pid == root) {
                        expected.push(root);
                    }
                    while let Some(pid) = walk.pop() {
                        let parent = all.iter().find(|p| p.pid == pid);
                        for p in all.iter().filter(|p| p.ppid == pid) {
                            if parent.is_some_and(|s| s.start_ticks > p.start_ticks)
                                || !visited.insert(p.pid)
                            {
                                continue;
                            }
                            expected.push(p.pid);
                            walk.push(p.pid);
                        }
                    }
                }
                let mut unique = std::collections::HashSet::new();
                expected.retain(|pid| unique.insert(*pid));
                let (tree, members) = process_forest(roots, &all);
                assert_eq!(tree.iter().map(|p| p.pid).collect::<Vec<_>>(), expected);
                for p in &all {
                    assert_eq!(members.contains(&p.pid), unique.contains(&p.pid));
                }
            }
        }
    }

    #[test]
    fn sampled_cpu_preserves_survivor_work_when_other_child_exits() {
        let mut tracked = HashMap::new();
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 1, 100), cpu_stat(2, 2, 10)], &mut tracked,),
            110
        );
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(2, 2, 20)], &mut tracked),
            10
        );
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(2, 2, 30), cpu_stat(3, 3, 5)], &mut tracked,),
            15
        );
    }

    #[test]
    fn sampled_cpu_missing_snapshot_does_not_count_lifetime_again() {
        let mut tracked = HashMap::new();
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 1, 100)], &mut tracked),
            100
        );
        assert_eq!(sampled_tree_cpu_delta(&[], &mut tracked), 0);
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 1, 110)], &mut tracked),
            10
        );
    }

    #[test]
    fn sampled_cpu_distinguishes_pid_reuse_and_overlapping_roots() {
        let mut tracked = HashMap::new();
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 1, 100)], &mut tracked),
            100
        );
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 2, 5), cpu_stat(1, 2, 5)], &mut tracked,),
            5
        );
        assert_eq!(tracked.len(), 2);
    }

    #[test]
    fn sampled_cpu_counter_regression_does_not_create_later_spike() {
        let mut tracked = HashMap::new();
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 1, 200)], &mut tracked),
            200
        );
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 1, 100)], &mut tracked),
            0
        );
        assert_eq!(
            sampled_tree_cpu_delta(&[cpu_stat(1, 1, 210)], &mut tracked),
            10
        );
    }

    #[test]
    fn required_toolchain_never_accepts_missing_or_substituted_compiler() {
        assert!(require_selected_toolchain(Some("1.99.0"), Some("1.99.0")).is_ok());
        assert!(require_selected_toolchain(Some("1.99.0"), None).is_err());
        assert!(require_selected_toolchain(Some("1.99.0"), Some("jr-abcdef1")).is_err());
        assert!(require_selected_toolchain(None, None).is_ok());
    }

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn finds_subcommand_after_global_flags() {
        assert_eq!(
            subcommand(&os(&[
                "+nightly", "-q", "--color", "never", "test", "-p", "x"
            ]))
            .as_deref(),
            Some("test")
        );
        assert_eq!(
            subcommand(&os(&["-Z", "unstable-options", "build"])).as_deref(),
            Some("build")
        );
        assert_eq!(subcommand(&os(&["--version"])), None);
    }

    #[test]
    fn only_records_build_like_commands() {
        assert!(RECORDED.contains(&"test"));
        assert!(!RECORDED.contains(&"metadata"));
        assert!(!RECORDED.contains(&"fmt"));
    }

    #[test]
    fn progress_config_goes_after_toolchain() {
        let a = with_progress_config(&os(&["+nightly", "test"]));
        assert_eq!(a[0], "+nightly");
        assert_eq!(a[1], "--config");
        assert_eq!(a.last().unwrap(), "test");
        assert_eq!(with_progress_config(&os(&["check"]))[0], "--config");
    }

    #[test]
    fn strips_ansi_sequences() {
        assert_eq!(
            strip_ansi("\x1b[1m\x1b[32m   Compiling\x1b[0m foo"),
            "   Compiling foo"
        );
    }

    #[test]
    fn classifies_processes() {
        assert_eq!(
            classify_proc("rustc", "/usr/bin/rustc --crate-name x"),
            "rustc"
        );
        assert_eq!(classify_proc("mold", "mold -o x"), "linker");
        assert_eq!(
            classify_proc(
                "jcode_desktop_u",
                "/home/j/p/target/debug/deps/jcode_desktop_ui-abc --quiet"
            ),
            "target-binary"
        );
        assert_eq!(
            classify_proc("build-script-bu", "/t/debug/build/x-1/build-script-build"),
            "build-script"
        );
    }
}
