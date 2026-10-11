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
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Cargo subcommands worth recording. Everything else (metadata, fmt, tree,
/// --version, ...) is passed straight through with no overhead.
const RECORDED: &[&str] = &[
    "build", "b", "check", "c", "test", "t", "bench", "run", "r", "clippy", "doc", "d", "rustc",
    "rustdoc", "nextest", "fix", "miri", "llvm-cov", "insta",
];

const SAMPLE_INTERVAL: Duration = Duration::from_millis(200);

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
}

#[derive(Debug, Serialize, Deserialize, Clone, Default)]
pub struct GitInfo {
    pub root: Option<String>,
    pub head: Option<String>,
    pub dirty_files: Option<usize>,
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

static CHILD_PID: AtomicI32 = AtomicI32::new(0);

extern "C" fn forward_signal(sig: libc::c_int) {
    let pid = CHILD_PID.load(Ordering::SeqCst);
    if pid > 0 {
        // SAFETY: kill is async-signal-safe.
        unsafe {
            libc::kill(pid, sig);
        }
    }
}

fn install_signal_forwarding() {
    // SAFETY: installing a handler that only calls async-signal-safe functions.
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = forward_signal as *const () as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);
        for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT] {
            libc::sigaction(sig, &sa, std::ptr::null_mut());
        }
        // The terminal already delivers SIGINT to cargo (same process group).
        // Ignore it here so the summary still gets written.
        libc::signal(libc::SIGINT, libc::SIG_IGN);
    }
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
    let pinned = std::env::current_dir()
        .ok()
        .and_then(|d| crate::toolchain::resolve(&d).ok().flatten())
        .and_then(|p| crate::toolchain::installed(&p.spec).ok().flatten());
    let mut cmd;
    match &pinned {
        Some(tc) => {
            cmd = Command::new(tc.bin("cargo"));
            apply_toolchain_env(&mut cmd, tc);
        }
        None => {
            cmd = Command::new(
                paths::real_cargo().unwrap_or_else(|_| PathBuf::from("/usr/bin/cargo")),
            );
        }
    }
    let cargo = PathBuf::from(cmd.get_program());
    let err = cmd.args(args).exec();
    eprintln!("justrust: failed to exec {}: {err}", cargo.display());
    std::process::exit(127);
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
    if !should_record(&args) {
        exec_cargo(&args);
    }
    match record(&args, opts) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("justrust: recording disabled for this run: {e:#}");
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
    // SAFETY: isatty has no memory-safety preconditions.
    unsafe { libc::isatty(fd) == 1 }
}

fn term_width(fd: i32) -> Option<u16> {
    // SAFETY: winsize is plain data and only written by the ioctl.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        (libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0).then_some(ws.ws_col)
    }
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

fn record(args: &[OsString], opts: Options) -> Result<i32> {
    // A pinned toolchain (`justrust.toml`) replaces the system cargo and
    // rustc. Installed on first use. Fails open to the system toolchain.
    let pinned = crate::toolchain::for_build();
    let real_cargo = match &pinned {
        Some(tc) => tc.bin("cargo"),
        None => paths::real_cargo()?,
    };
    let shim = paths::ensure_rustc_shim()?;
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

    let id = new_run_id();
    let run_dir = paths::runs_dir()?.join(&id);
    std::fs::create_dir_all(&run_dir)?;

    // Agent runs get a private target dir so parallel agents do not block.
    let slot = opts.agent.then(|| slots::acquire(args)).flatten();
    let cargo_args = slot.as_ref().map_or(args, |s| s.args.as_slice());

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
        id: id.clone(),
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
        mem_total_mb: procfs::meminfo().map(|m| m.total_mb).unwrap_or(0),
        env,
        slot: slot.as_ref().map(|s| s.info.clone()),
        toolchain: pinned.as_ref().map(|tc| tc.id.clone()),
    };
    std::fs::write(run_dir.join("meta.json"), serde_json::to_vec_pretty(&meta)?)?;

    let git = std::thread::spawn(git_info);
    // Before cargo starts, so it and everything it runs share the scope.
    let scope = crate::sched::enter(&id);

    // The progress bar feeds `live.json` (`justrust status`). When the user
    // would not have seen it, ask for it with `--config` (unlike the env var,
    // that does not leak into test binaries that run cargo) and strip it again.
    let user_progress = tty && !opts.agent;
    let strip_progress = !user_progress && std::env::var_os("CARGO_TERM_PROGRESS_WHEN").is_none();
    let cargo_args = if strip_progress {
        with_progress_config(cargo_args)
    } else {
        cargo_args.to_vec()
    };

    let mut cmd = Command::new(&real_cargo);
    if let Some(tc) = &pinned {
        apply_toolchain_env(&mut cmd, tc);
        if tc.nightly {
            // Lets the shim time passes of local crates: on nightly, unstable
            // options never change the incremental cache key.
            cmd.env("JUSTRUST_TOOLCHAIN_NIGHTLY", "1");
        }
    }
    cmd.args(&cargo_args)
        .env("RUSTC", &shim)
        .env("JUSTRUST_REAL_RUSTC", &real_rustc)
        .env("JUSTRUST_RUN_DIR", &run_dir)
        .env("JUSTRUST_RUN_ID", &id)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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

    let start = paths::now();
    let mut child = cmd
        .spawn()
        .with_context(|| format!("spawning {}", real_cargo.display()))?;
    let pid = child.id() as i32;
    CHILD_PID.store(pid, Ordering::SeqCst);
    install_signal_forwarding();

    let lines: Arc<Mutex<Vec<OutputLine>>> = Arc::default();
    let out = child.stdout.take().context("stdout")?;
    let err = crate::live::SniffReader::new(
        child.stderr.take().context("stderr")?,
        crate::live::Sniffer::new(Some(&run_dir), strip_progress),
    );
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
    let t_out = spawn_tee(out, std::io::stdout(), "o", lines.clone(), f_out.clone());
    let t_err = spawn_tee(err, std::io::stderr(), "e", lines.clone(), f_err.clone());

    let done: Arc<(Mutex<bool>, std::sync::Condvar)> = Arc::default();
    let sampler = {
        let done = done.clone();
        std::thread::spawn(move || sample_loop(pid, &done))
    };

    let (code, cpu_user, cpu_sys) = wait_child(pid);
    let end = paths::now();
    let sched = scope.map(crate::sched::Scope::finish);
    CHILD_PID.store(0, Ordering::SeqCst);
    {
        let (lock, cv) = &*done;
        *lock.lock().unwrap() = true;
        cv.notify_all();
    }
    let _ = t_out.join();
    let _ = t_err.join();
    let (samples, procs) = sampler.join().unwrap_or_default();
    let git = git.join().unwrap_or_default();

    let lines = std::mem::take(&mut *lines.lock().unwrap());
    let mut hidden = agent_output::Hidden::default();
    for f in [&f_out, &f_err] {
        if let Some(f) = f.as_ref() {
            hidden.merge(f.lock().unwrap().hidden);
        }
    }
    if let Err(e) = finish(
        opts,
        hidden,
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
    ) {
        eprintln!("justrust: failed to write run summary: {e:#}");
    }
    if let Some(s) = &slot {
        if code != 0 {
            slots::heal_after_failure(&s.info, args, lines.iter().map(|l| l.l.as_str()));
        }
        // Detached and at most hourly; never delays or fails this run.
        slots::maybe_gc_background(&s.info);
    }
    Ok(code)
}

#[allow(clippy::too_many_arguments)]
fn finish(
    opts: Options,
    hidden: agent_output::Hidden,
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
) -> Result<()> {
    write_jsonl(&run_dir.join("output.jsonl"), lines)?;
    write_jsonl(&run_dir.join("samples.jsonl"), samples)?;
    std::fs::write(
        run_dir.join("processes.json"),
        serde_json::to_vec_pretty(procs)?,
    )?;
    let mut units = summary::load_units(run_dir);
    units.extend(summary::units_from_wrapper_procs(procs));
    let mut s = summary::build(
        meta, git, start, end, code, cpu_secs, lines, samples, procs, &units,
    );
    s.sched = sched;
    crate::split_index::attach(&mut s);
    std::fs::write(run_dir.join("summary.json"), serde_json::to_vec_pretty(&s)?)?;
    paths::append_line(
        &paths::index_file()?,
        &serde_json::to_string(&s.index_entry())?,
    )?;
    crate::split_index::record(&s);
    crate::split_index::maybe_refresh(&s);
    crate::depcache_gc::maybe_spawn();
    if opts.agent {
        eprint!("{}", s.agent_footer(&hidden));
    } else if std::env::var_os("JUSTRUST_QUIET").is_none() {
        eprintln!("justrust: {}", s.one_line());
    }
    Ok(())
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
/// cargo and every descendant it waited for (rustc, linkers, tests).
fn wait_child(pid: i32) -> (i32, f64, f64) {
    let mut status = 0;
    // SAFETY: rusage is plain data written by wait4.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: pointers to valid local storage.
        let r = unsafe { libc::wait4(pid, &mut status, 0, &mut ru) };
        if r == pid {
            break;
        }
        if r == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return (1, 0.0, 0.0);
    }
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    };
    // ru only covers the cargo process itself. Children totals come from getrusage.
    // SAFETY: as above.
    let mut kids: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe {
        libc::getrusage(libc::RUSAGE_CHILDREN, &mut kids);
    }
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    (code, tv(kids.ru_utime), tv(kids.ru_stime))
}

fn spawn_tee<R: Read + Send + 'static, W: Write + Send + 'static>(
    mut src: R,
    mut dst: W,
    stream: &'static str,
    lines: Arc<Mutex<Vec<OutputLine>>>,
    filter: Arc<Option<Mutex<agent_output::Filter>>>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        if let Some(filter) = filter.as_ref() {
            return tee_filtered(src, dst, stream, &lines, filter);
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
            let _ = dst.write_all(&buf[..n]);
            let _ = dst.flush();
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
    lines: &Mutex<Vec<OutputLine>>,
    filter: &Mutex<agent_output::Filter>,
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
        if filter.lock().unwrap().keep(&text) {
            let _ = dst.write_all(text.as_bytes());
            let _ = dst.write_all(b"\n");
            let _ = dst.flush();
        }
        push_line(lines, paths::now(), stream, body);
    }
}

fn push_line(lines: &Mutex<Vec<OutputLine>>, t: f64, stream: &str, raw: &[u8]) {
    // Progress bars rewrite the line with '\r'. Keep only the final segment.
    let raw = raw
        .rsplit(|&b| b == b'\r')
        .find(|seg| !seg.is_empty())
        .unwrap_or(&[]);
    let text = strip_ansi(&String::from_utf8_lossy(raw));
    if text.trim().is_empty() {
        return;
    }
    lines.lock().unwrap().push(OutputLine {
        t,
        s: stream.to_owned(),
        l: text,
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
    let at = usize::from(
        args.first()
            .is_some_and(|a| a.to_string_lossy().starts_with('+')),
    );
    let mut out = args[..at].to_vec();
    for c in ["term.progress.when=\"always\"", "term.progress.width=200"] {
        out.push("--config".into());
        out.push(c.into());
    }
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
    if comm.starts_with("build-script") || exe.contains("/build/") && exe.contains("build-script") {
        return "build-script";
    }
    if exe.contains("/target/") || exe.contains("/deps/") {
        return "target-binary";
    }
    if exe.ends_with("/rustc") || cmdline.contains("rustc-parallel-frontend") {
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

fn sample_loop(
    root: i32,
    done: &(Mutex<bool>, std::sync::Condvar),
) -> (Vec<Sample>, Vec<ProcRecord>) {
    let hz = procfs::clk_tck();
    let page_mb = procfs::page_size() as f64 / 1_048_576.0;
    let ncpu = procfs::ncpu() as f64;
    let mut samples = Vec::new();
    let mut tracked: HashMap<(i32, u64), Tracked> = HashMap::new();
    let mut prev_cpu = procfs::cpu_times();
    let mut prev_psi = procfs::pressure();
    let mut prev_t = paths::now();
    let mut prev_tree_ticks: u64 = 0;
    loop {
        let stop = *done.0.lock().unwrap();
        let t = paths::now();
        let all: Vec<procfs::ProcStat> = procfs::all_pids()
            .into_iter()
            .filter_map(procfs::read_stat)
            .collect();
        let tree = tree_of(root, &all);
        let mut tree_ticks = 0;
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
            entry.last_cpu_ticks = p.cpu_ticks;
            let rec = entry.rec.as_mut().unwrap();
            rec.last_seen = t;
            rec.cpu_secs = p.cpu_ticks as f64 / hz;
            rec.max_rss_mb = rec.max_rss_mb.max(p.rss_pages as f64 * page_mb);
            if rec.kind == "rustc" {
                rustc_procs += 1;
            }
            tree_ticks += p.cpu_ticks;
            rss += p.rss_pages;
        }
        let in_tree: std::collections::HashSet<i32> = tree.iter().map(|p| p.pid).collect();
        let foreign = all
            .iter()
            .filter(|p| p.comm == "rustc" && !in_tree.contains(&p.pid))
            .count();

        let cpu = procfs::cpu_times();
        let psi = procfs::pressure();
        let dt = (t - prev_t).max(1e-3);
        let sys_cores = match (prev_cpu, cpu) {
            (Some(a), Some(b)) if b.total > a.total => {
                (b.busy - a.busy) as f64 / (b.total - a.total) as f64 * ncpu
            }
            _ => 0.0,
        };
        // Tree ticks can drop when processes exit, so clamp at zero.
        let build_cores = (tree_ticks.saturating_sub(prev_tree_ticks)) as f64 / hz / dt;
        if !samples.is_empty() || stop {
            samples.push(Sample {
                t,
                sys_cores,
                build_cores: build_cores.min(ncpu),
                build_rss_mb: rss as f64 * page_mb,
                mem_available_mb: procfs::meminfo().map(|m| m.available_mb).unwrap_or(0),
                rustc_procs,
                foreign_rustc_procs: foreign,
                psi_cpu_ms: psi.cpu_us.saturating_sub(prev_psi.cpu_us) as f64 / 1000.0,
                psi_mem_ms: psi.memory_us.saturating_sub(prev_psi.memory_us) as f64 / 1000.0,
                psi_io_ms: psi.io_us.saturating_sub(prev_psi.io_us) as f64 / 1000.0,
            });
        } else {
            // First pass only establishes baselines.
            samples.push(Sample {
                t,
                sys_cores: 0.0,
                build_cores: 0.0,
                build_rss_mb: rss as f64 * page_mb,
                mem_available_mb: procfs::meminfo().map(|m| m.available_mb).unwrap_or(0),
                rustc_procs,
                foreign_rustc_procs: foreign,
                psi_cpu_ms: 0.0,
                psi_mem_ms: 0.0,
                psi_io_ms: 0.0,
            });
        }
        prev_cpu = cpu;
        prev_psi = psi;
        prev_t = t;
        prev_tree_ticks = tree_ticks;
        if stop {
            break;
        }
        // Sleep until the next sample, but wake as soon as the build ends.
        let guard = done.0.lock().unwrap();
        let _ = done
            .1
            .wait_timeout_while(guard, SAMPLE_INTERVAL, |finished| !*finished);
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

fn tree_of(root: i32, all: &[procfs::ProcStat]) -> Vec<procfs::ProcStat> {
    let mut children: HashMap<i32, Vec<usize>> = HashMap::new();
    for (i, s) in all.iter().enumerate() {
        children.entry(s.ppid).or_default().push(i);
    }
    let mut out = Vec::new();
    if let Some(r) = all.iter().find(|s| s.pid == root) {
        out.push(r.clone());
    }
    let mut stack = vec![root];
    while let Some(pid) = stack.pop() {
        for &i in children.get(&pid).into_iter().flatten() {
            out.push(all[i].clone());
            stack.push(all[i].pid);
        }
    }
    out
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
    GitInfo {
        root: run(&["rev-parse", "--show-toplevel"]),
        head: run(&["rev-parse", "--short=12", "HEAD"]),
        dirty_files: run(&["status", "--porcelain", "--untracked-files=no"])
            .map(|s| s.lines().count()),
    }
}

/// Resolve dependency features once for the whole workspace instead of per
/// command, so `-p a`, `-p b`, and `--workspace` compile the same build of
/// each dependency. Without it, switching packages rebuilds dependencies
/// with different feature sets (27 dirty units going from `-p
/// jcode-desktop-harness` to `-p jcode-desktop-model`), and every variant
/// is a separate depcache entry. Measured in FINDINGS.md section 18.
///
/// Uses cargo's `-Zfeature-unification` (`resolver.feature-unification =
/// "workspace"`). The nightly gate is opened with cargo's channel override
/// rather than `RUSTC_BOOTSTRAP`, because rustc sees `RUSTC_BOOTSTRAP`, so
/// cargo fingerprints it and setting it would rebuild everything once.
/// The override only affects cargo, not rustc.
///
/// Off with `JUSTRUST_UNIFY_FEATURES=0`. Respects an explicit
/// `CARGO_RESOLVER_FEATURE_UNIFICATION` (`selected` restores cargo's default).
fn unify_features(cmd: &mut Command) {
    if matches!(
        std::env::var("JUSTRUST_UNIFY_FEATURES").as_deref(),
        Ok("0") | Ok("off") | Ok("false") | Ok("no")
    ) || std::env::var_os("CARGO_RESOLVER_FEATURE_UNIFICATION").is_some()
    {
        return;
    }
    cmd.env("CARGO_RESOLVER_FEATURE_UNIFICATION", "workspace")
        .env("CARGO_UNSTABLE_FEATURE_UNIFICATION", "true");
    if std::env::var_os("RUSTC_BOOTSTRAP").is_none() {
        cmd.env("__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS", "nightly");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
