//! The resident build daemon: `justrust __buildd`.
//!
//! One per user and justrust build, started on demand by the first agent
//! build that wants it, exits after 15 minutes without requests or running
//! builds. Agent builds (`justrust check|test|build|clippy`) hand their
//! request to it over `~/.justrust/buildd/<build>.sock` instead of running
//! cargo themselves. Clients of different builds (a pinned CI client and a
//! newer developer client on one hosted machine, or the client before and
//! after an update) each talk to a daemon of their own build:
//!
//! - **Coalescing.** A request is identified by its working directory, its
//!   full argv, the environment that can change the result (everything
//!   except agent identity variables such as `JCODE_SESSION_ID`, see
//!   [`IGNORED_ENV`]), and a stamp of the source state (path, size, mtime of
//!   every file under the workspace and its path dependencies, plus cargo
//!   config). A request equal to a build already in flight attaches to it:
//!   it gets the whole output from the start, then the rest as it comes,
//!   and the same exit code. Two agents asking for the same test run share
//!   one cargo invocation instead of the second waiting for the first's slot
//!   and then running a no-op cargo and the tests again.
//! - **Execution is unchanged.** The daemon runs `justrust <sub> <args>` (the
//!   same binary) with the client's exact environment and working directory,
//!   so routing, slots, the pinned toolchain, recording and the agent report
//!   all behave as without the daemon. Output bytes and exit code are the
//!   child's. The slot owner is the client's session (`JUSTRUST_SLOT_SESSION`).
//! - **Prebuild seam.** A `prebuild` request starts the same kind of build
//!   with nobody attached and keeps it running when nobody listens, so a
//!   watcher (or an editor hook: `justrust daemon prebuild -- check -p x`)
//!   can start the build an agent is about to ask for. The agent's request
//!   then attaches to it when the sources did not change in between.
//!
//! Fail open: no daemon, a daemon that cannot start, a protocol error, or a
//! daemon from another justrust build before any output arrived means the
//! build runs in the client as before. On by default for non-terminal
//! agent builds on Linux; `JUSTRUST_BUILDD=0` disables the daemon,
//! `JUSTRUST_BUILDD_PREBUILD=0` disables speculative prebuilds.
//! Experimental `JUSTRUST_BUILDD_NATIVE_EMPTY=1` skips source validation only
//! when no active or finished candidate exists. Native Cargo still runs, and
//! the unvalidated execution can never be attached to or replayed.
//! Foreground check and clippy skip validation when no same-key owner/result
//! or same-session wrong guess needs coordination. `JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE=0`
//! disables this; `1` extends it to other commands. Cargo always runs.
//!
//! Wire format, both directions: `[u8 kind][u32 len, big endian][payload]`.
//! Client to daemon: `R` request JSON, `P` ping, `S` shutdown. Daemon to
//! client: `O`/`E` stdout/stderr bytes, `X` exit JSON, `I` info JSON (pong
//! and prebuild acks), `F` refusal (the client builds itself).

use crate::sys::Detach;
use crate::sys::net::{UnixListener, UnixStream};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

mod warm;

pub const DAEMON_ARG: &str = "__buildd";
/// Set on the builds the daemon runs: they never go back to the daemon.
const CHILD_ENV: &str = "JUSTRUST_BUILDD_CHILD";
/// Set on a prebuild: a guess nobody waits for. It routes like the request
/// it copies (hosted when that is where the request goes), never falls back
/// to a local compile after a remote failure, never compiles locally on
/// battery, and does not bring compile-only test executables back.
pub const GUESS_ENV: &str = "JUSTRUST_BUILDD_GUESS";

/// Set on a build whose units a compile-only prebuild compiled first:
/// `<prebuild run id> <seconds this request waited for it>`. That build
/// finds the units Fresh and would otherwise report that nothing was
/// compiled, with the wait in nobody's record.
pub const WAITED_ENV: &str = "JUSTRUST_BUILDD_WAITED";

fn waited_env(run_id: Option<&str>, secs: f64) -> Option<String> {
    let id = run_id?;
    let ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    (ok && secs.is_finite() && secs >= 0.0).then(|| format!("{id} {secs:.3}"))
}

/// The prebuild whose units this process's build reuses: its run id and the
/// seconds the daemon held this request back for it. Taken out of the
/// environment on first use, so call it before anything is spawned: the
/// note must never reach cargo, a compiler or a cache key.
pub fn waited_prebuild() -> Option<(String, f64)> {
    static NOTE: std::sync::OnceLock<Option<(String, f64)>> = std::sync::OnceLock::new();
    NOTE.get_or_init(|| {
        let v = std::env::var(WAITED_ENV).ok()?;
        // SAFETY: single-threaded where the recorder first asks.
        unsafe { std::env::remove_var(WAITED_ENV) };
        parse_waited(&v)
    })
    .clone()
}

/// Set on every build the daemon starts: its stamp of the sources the build
/// was started for. A slot records it, so the same command started after an
/// edit does not wait for a build of the old sources (`slots`).
pub const STAMP_ENV: &str = "JUSTRUST_BUILDD_STAMP";

/// This process's [`STAMP_ENV`]. Taken out of the environment on first use,
/// like [`waited_prebuild`], and for the same reason.
pub fn build_stamp() -> Option<String> {
    static STAMP: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    STAMP
        .get_or_init(|| {
            let v = std::env::var(STAMP_ENV).ok()?;
            // SAFETY: single-threaded where the recorder first asks.
            unsafe { std::env::remove_var(STAMP_ENV) };
            let ok = !v.is_empty() && v.bytes().all(|b| b.is_ascii_alphanumeric());
            ok.then_some(v)
        })
        .clone()
}

fn parse_waited(v: &str) -> Option<(String, f64)> {
    let (id, secs) = v.split_once(' ')?;
    let secs: f64 = secs.parse().ok()?;
    let ok = !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    (ok && secs.is_finite() && secs >= 0.0).then(|| (id.to_owned(), secs))
}
const IDLE_EXIT: Duration = Duration::from_secs(900);

/// Variables that identify the agent or its terminal, not the build. Two
/// agents whose environments differ only in these share a build.
const IGNORED_ENV: &[&str] = &[
    "JCODE_SESSION_ID",
    "JCODE_SOCKET",
    "JCODE_ACTIVE_PROVIDER",
    "JCODE_RUNTIME_PROVIDER",
    "JCODE_INITIAL_PROVIDER_EXPLICIT",
    "JCODE_DEFERRED_AUTH_BOOTSTRAP",
    "JCODE_NON_INTERACTIVE",
    "JCODE_DEBUG_CONTROL",
    "JCODE_CLIENT_SELFDEV_MODE",
    "JCODE_RESUMING",
    "JCODE_REPO_DIR",
    "JCODE_NO_AUTO_UPDATE",
    "JCODE_SCRATCH_DIR",
    "DESKTOP_STARTUP_ID",
    "INVOCATION_ID",
    "JOURNAL_STREAM",
    "SYSTEMD_EXEC_PID",
    "MANAGERPID",
    "MANAGERPIDFDID",
    "WINDOWID",
    "PWD",
    "OLDPWD",
    "SHLVL",
    "_",
    "JUSTRUST_SLOT_SESSION",
    // The connection a hosted client arrived on, source port included: it
    // differs for every client and after every reconnect of the same one,
    // and says nothing about the build.
    "SSH_CLIENT",
    "SSH_CONNECTION",
    "SSH_TTY",
    // Benchmarks pin run ids; an attached request links its id to the
    // shared run instead (`Exit::run_id`).
    "JUSTRUST_RUN_ID_OVERRIDE",
    // The daemon's own note to a build; never part of what was asked.
    WAITED_ENV,
    STAMP_ENV,
];
const IGNORED_ENV_PREFIXES: &[&str] = &["KITTY_", "JCODE_SPAWN_", "JCODE_TUI_", "JCODE_RELOAD_"];

fn env_ignored(name: &str) -> bool {
    IGNORED_ENV.contains(&name) || IGNORED_ENV_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// This process is a build the daemon runs: an early answer can end the
/// client's request while the build goes on (`early::EXIT_MARKER`).
pub fn is_child() -> bool {
    std::env::var_os(CHILD_ENV).is_some()
}

/// Exit code of a guess that declined to build (no remote, or it would have
/// compiled locally on battery). The daemon never hands it to a request as
/// a result: the request builds for itself.
pub const GUESS_DROPPED: i32 = 75;

/// A prebuild that declined to run.
fn dropped_guess(b: &Build, l: &Log) -> bool {
    l.exit == Some(GUESS_DROPPED) && *b.prebuild.lock().unwrap()
}

/// Exit codes of a process ended by SIGHUP, SIGINT, SIGKILL or SIGTERM.
const SIGNAL_EXITS: [i32; 4] = [129, 130, 137, 143];

/// The daemon stopped this build (a wrong guess, older sources, nobody left
/// waiting). Marked before the signal, so that no request can take a build
/// that is about to die for one that is running.
fn stop(b: &Build) {
    b.stopped.store(true, std::sync::atomic::Ordering::SeqCst);
    // Never pid 0: that is this process's own group.
    if b.pid > 0 {
        crate::sys::terminate_group(b.pid, false);
    }
}

fn stopped(b: &Build) -> bool {
    b.stopped.load(std::sync::atomic::Ordering::SeqCst)
}

/// Not the outcome of building these sources, whatever its exit code says:
/// a guess that declined to run, a build the daemon stopped, or a prebuild
/// nobody had asked for that a signal ended. A kill says nothing about the
/// sources, so the request builds for itself.
fn no_result(b: &Build, l: &Log) -> bool {
    dropped_guess(b, l)
        || stopped(b)
        || (*b.prebuild.lock().unwrap()
            && !l.claimed
            && l.exit.is_some_and(|c| SIGNAL_EXITS.contains(&c)))
}

/// This process is a prebuild: started on a guess, with nobody waiting.
pub fn is_guess() -> bool {
    std::env::var_os(GUESS_ENV).is_some()
}

/// On by default; `JUSTRUST_BUILDD=0` builds in the client as before.
///
/// `justrust local off` does not disable the daemon: its builds route like
/// any other, so they (and their prebuilds) run on hosted machines, and the
/// rustc shim still refuses every local compile.
pub fn enabled() -> bool {
    // CI runs and test harnesses would leave a daemon behind per job.
    if std::env::var_os("CI").is_some() && std::env::var_os("JUSTRUST_BUILDD").is_none() {
        return false;
    }
    !matches!(
        std::env::var("JUSTRUST_BUILDD").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}

fn dir() -> Result<PathBuf> {
    Ok(crate::paths::home()?.join("buildd"))
}

/// File stem of this build's daemon socket and lock. A daemon only serves
/// clients of its own build, so each build gets its own pair: two builds
/// sharing one pair evicted each other's daemon on every alternating request,
/// and while the evicted one drained its builds nobody could start a daemon
/// at all (each request then waited out the 3 s start timeout).
fn build_stem(build: &str) -> String {
    match build.strip_prefix("sha256-") {
        Some(hex) if hex.len() >= 16 && hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
            hex[..16].to_owned()
        }
        // An unreadable binary has a per-process id: nothing to share.
        _ => "unidentified".to_owned(),
    }
}

fn stem() -> String {
    build_stem(&crate::remote_daemon::build_id())
}

pub fn sock_path() -> Result<PathBuf> {
    Ok(dir()?.join(format!("{}.sock", stem())))
}

fn lock_path() -> Result<PathBuf> {
    Ok(dir()?.join(format!("{}.lock", stem())))
}

/// Locks and sockets of builds that have had no daemon for this long go.
const STALE_BUILD_FILES: Duration = Duration::from_secs(24 * 3600);

/// Remove the socket and lock files daemons of other builds left behind.
/// A lock is only removed while this process holds it, and a starting daemon
/// checks that the lock it took is still the file at its path (`daemon`), so
/// no two daemons of one build can result.
fn sweep_stale(d: &Path, mine: &Path, older_than: Duration) {
    let Ok(rd) = std::fs::read_dir(d) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p == mine || p.extension().is_none_or(|x| x != "lock") {
            continue;
        }
        let old = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= older_than);
        if !old {
            continue;
        }
        let Ok(f) = std::fs::OpenOptions::new().write(true).open(&p) else {
            continue;
        };
        if f.try_lock().is_ok() {
            let _ = std::fs::remove_file(p.with_extension("sock"));
            let _ = std::fs::remove_file(&p);
        }
    }
}

// ------------------------------------------------------------------ frames

fn write_msg<W: Write>(w: &mut W, kind: u8, payload: &[u8]) -> std::io::Result<()> {
    let mut buf = Vec::with_capacity(5 + payload.len());
    buf.push(kind);
    buf.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    buf.extend_from_slice(payload);
    w.write_all(&buf)?;
    w.flush()
}

/// `Ok(None)` on a clean end of stream.
fn read_msg<R: Read>(r: &mut R) -> std::io::Result<Option<(u8, Vec<u8>)>> {
    let mut h = [0u8; 5];
    match r.read_exact(&mut h) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let n = u32::from_be_bytes([h[1], h[2], h[3], h[4]]) as usize;
    if n > 256 << 20 {
        return Err(std::io::Error::other("frame too large"));
    }
    let mut p = vec![0u8; n];
    r.read_exact(&mut p)?;
    Ok(Some((h[0], p)))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Request {
    /// Client's justrust build (`remote_daemon::build_id`): a daemon from
    /// another build refuses, so the client never runs an older justrust.
    pub build: String,
    pub cwd: String,
    /// `sub args...`, route flags already turned into JUSTRUST_REMOTE.
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Start the build and return at once; keep it running without clients.
    #[serde(default)]
    pub prebuild: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Exit {
    pub code: i32,
    /// This request joined a build that was already running.
    pub attached: bool,
    /// Seconds the build had been running when this request arrived.
    pub joined_after: f64,
    /// Requests that shared this build so far, this one included.
    pub sharers: usize,
    /// Daemon time to answer (stamp and lookup), milliseconds.
    pub lookup_ms: f64,
    /// The shared build's pinned run id, when its starter pinned one.
    #[serde(default)]
    pub run_id: Option<String>,
    /// It was a prebuild that had already finished: its result was replayed.
    #[serde(default)]
    pub replayed: bool,
    /// Answered early from rust-analyzer (`JUSTRUST_EARLY_EXIT`): the build
    /// keeps running in the daemon, nobody waits for it.
    #[serde(default)]
    pub early: bool,
    /// The replayed build is one that continued after an early answer.
    #[serde(default)]
    pub after_early: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Info {
    pub build: String,
    pub pid: u32,
    pub running: usize,
    pub served: u64,
    pub coalesced: u64,
    /// Prebuild ack: whether it started a build or found one running.
    #[serde(default)]
    pub attached: bool,
}

// ------------------------------------------------------------------ client

fn isatty(fd: i32) -> bool {
    crate::sys::isatty(fd)
}

/// Build the request for `full` (`sub args...`) from this process, or
/// `None` when this build must not go through the daemon.
fn request_for(full: &[OsString], prebuild: bool) -> Option<Request> {
    let argv: Vec<String> = full
        .iter()
        .map(|a| a.to_str().map(str::to_owned))
        .collect::<Option<_>>()?;
    let sub = argv.first()?;
    if !matches!(
        sub.as_str(),
        "check" | "c" | "test" | "t" | "build" | "b" | "clippy"
    ) {
        return None;
    }
    let mut env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .collect();
    // The slot belongs to the agent that asked, not to the daemon's session.
    if std::env::var_os("JUSTRUST_SLOT_SESSION").is_none() {
        env.push(("JUSTRUST_SLOT_SESSION".into(), crate::slots::session_key()));
    }
    env.sort();
    Some(Request {
        build: crate::remote_daemon::build_id(),
        cwd: std::env::current_dir().ok()?.to_str()?.to_owned(),
        argv,
        env,
        prebuild,
    })
}

/// Agent builds: run `full` through the daemon when opted in. Returns the
/// exit code, or `None` when the build should run here (not opted in, not
/// eligible, or any daemon problem before output started).
pub fn maybe_run(full: &[OsString]) -> Option<i32> {
    if !cfg!(target_os = "linux")
        || !enabled()
        || std::env::var_os(CHILD_ENV).is_some()
        || std::env::var_os("JUSTRUST_RUN_DIR").is_some()
        // A terminal gets cargo's progress bar and colours from a build
        // that sees it; through the daemon it would see pipes.
        || isatty(0)
        || isatty(1)
        || isatty(2)
    {
        return None;
    }
    let req = request_for(full, false)?;
    match run_request(&req) {
        Ok(code) => Some(code),
        Err(Fallback::Before(e)) => {
            if std::env::var_os("JUSTRUST_BUILDD_EXPLAIN").is_some() {
                eprintln!("justrust: build daemon unavailable, building here: {e:#}");
            }
            None
        }
        Err(Fallback::After(e)) => {
            eprintln!(
                "justrust: lost the build daemon mid-build ({e:#}); running the build here, \
                 output follows again from the start"
            );
            None
        }
    }
}

enum Fallback {
    /// Nothing was printed: building here is invisible to the caller.
    Before(anyhow::Error),
    /// Output already reached the caller.
    After(anyhow::Error),
}

fn connect() -> Result<UnixStream> {
    let sock = sock_path()?;
    if let Ok(s) = UnixStream::connect(&sock) {
        return Ok(s);
    }
    spawn()?;
    let t = Instant::now();
    loop {
        if let Ok(s) = UnixStream::connect(&sock) {
            return Ok(s);
        }
        if t.elapsed() > Duration::from_secs(3) {
            bail!("build daemon did not start (see ~/.justrust/buildd/daemon.log)");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn run_request(req: &Request) -> std::result::Result<i32, Fallback> {
    let before = Fallback::Before;
    let mut s = connect().map_err(before)?;
    let body = serde_json::to_vec(req).map_err(|e| before(e.into()))?;
    write_msg(&mut s, b'R', &body).map_err(|e| before(e.into()))?;
    let mut out = std::io::stdout().lock();
    let mut err = std::io::stderr();
    let mut printed = false;
    loop {
        let m = read_msg(&mut s).map_err(|e| {
            if printed {
                Fallback::After(e.into())
            } else {
                Fallback::Before(e.into())
            }
        })?;
        match m {
            Some((b'O', p)) => {
                printed = true;
                let _ = out.write_all(&p);
                let _ = out.flush();
            }
            Some((b'E', p)) => {
                printed = true;
                let _ = out.flush();
                let _ = err.write_all(&p);
            }
            Some((b'X', p)) => {
                let x: Exit = serde_json::from_slice(&p).unwrap_or(Exit {
                    code: 1,
                    ..Default::default()
                });
                let _ = out.flush();
                if x.early {
                    eprintln!(
                        "justrust: returned early on rust-analyzer's errors (JUSTRUST_EARLY_EXIT=1); \
                         cargo continues in the background: the next identical run replays its \
                         verdict, `justrust log` has it once it finishes"
                    );
                } else if x.replayed && x.after_early {
                    link_run(x.run_id.as_deref());
                    eprintln!(
                        "justrust: rustc's verdict from the build that continued after the early \
                         answer, finished {:.1}s before you asked",
                        -x.joined_after
                    );
                } else if x.replayed {
                    link_run(x.run_id.as_deref());
                    eprintln!(
                        "justrust: result of a prebuild of these exact sources, finished {:.1}s \
                         before you asked (JUSTRUST_BUILDD_PREBUILD=0 disables)",
                        -x.joined_after
                    );
                } else if x.attached {
                    link_run(x.run_id.as_deref());
                    eprintln!(
                        "justrust: shared an identical build that was already running \
                         ({:.1}s in, {} requests share it; JUSTRUST_BUILDD=0 disables)",
                        x.joined_after, x.sharers
                    );
                }
                return Ok(x.code);
            }
            Some((b'I', _)) => {}
            Some((b'F', p)) => {
                let why = String::from_utf8_lossy(&p).into_owned();
                return Err(if printed {
                    Fallback::After(anyhow::anyhow!(why))
                } else {
                    Fallback::Before(anyhow::anyhow!(why))
                });
            }
            Some((k, _)) => {
                let e = anyhow::anyhow!("unexpected frame {k}");
                return Err(if printed {
                    Fallback::After(e)
                } else {
                    Fallback::Before(e)
                });
            }
            None => {
                let e = anyhow::anyhow!("daemon closed the connection");
                return Err(if printed {
                    Fallback::After(e)
                } else {
                    Fallback::Before(e)
                });
            }
        }
    }
}

/// An attached request with a pinned run id (benchmarks) gets that id as a
/// link to the shared run, so `justrust show <id>` and summaries work.
fn link_run(shared: Option<&str>) {
    let (Some(shared), Ok(mine)) = (shared, std::env::var("JUSTRUST_RUN_ID_OVERRIDE")) else {
        return;
    };
    let ok = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    if mine == shared || !ok(&mine) || !ok(shared) {
        return;
    }
    if let Ok(d) = crate::paths::runs_dir() {
        let _ = crate::sys::symlink(Path::new(shared), &d.join(&mine));
    }
}

fn live_exe() -> Result<PathBuf> {
    let exe = std::env::current_exe().context("locating current justrust executable")?;
    Ok(resolve_live_exe(&exe))
}

fn resolve_live_exe(exe: &Path) -> PathBuf {
    // On Linux, /proc/self/exe appends " (deleted)" when the running binary is
    // atomically replaced (e.g. during release activation or auto-update).
    if let Some(s) = exe.to_str()
        && let Some(stripped) = s.strip_suffix(" (deleted)")
    {
        let candidate = PathBuf::from(stripped);
        if candidate.is_file() {
            return candidate;
        }
    }
    exe.to_path_buf()
}

fn current_exe_identity() -> Option<(PathBuf, (u64, u64))> {
    let exe = std::env::current_exe().ok()?;
    let md = std::fs::metadata(&exe).ok()?;
    Some((exe, crate::sys::file_id(&md)))
}

fn exe_is_replaced(initial: Option<&(PathBuf, (u64, u64))>, current: Option<&Path>) -> bool {
    if let Some(cur) = current
        && cur.to_str().is_some_and(|s| s.ends_with(" (deleted)"))
    {
        return true;
    }
    if let Some((path, id)) = initial {
        return !std::fs::metadata(path).is_ok_and(|m| crate::sys::file_id(&m) == *id);
    }
    false
}

fn spawn() -> Result<()> {
    let d = dir()?;
    std::fs::create_dir_all(&d)?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(d.join("daemon.log"))?;
    let mut c = Command::new(live_exe()?);
    c.arg(DAEMON_ARG)
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // Own process group: survives the agent's Ctrl+C.
    c.detached();
    crate::procfs::close_inherited_fds(&mut c);
    c.spawn().context("starting the build daemon")?;
    Ok(())
}

fn info_request(kind: u8) -> Result<Info> {
    let mut s = UnixStream::connect(sock_path()?).context("no build daemon running")?;
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    write_msg(&mut s, kind, &[])?;
    match read_msg(&mut s)? {
        Some((b'I', p)) => Ok(serde_json::from_slice(&p)?),
        _ => bail!("bad answer from the build daemon"),
    }
}

/// `justrust daemon status|stop|prebuild ARGS...`.
pub fn command(action: &str, args: Vec<OsString>) -> Result<()> {
    match action {
        "status" => match info_request(b'P') {
            Ok(i) => println!(
                "build daemon pid {}, {} running, {} requests served, {} coalesced{}",
                i.pid,
                i.running,
                i.served,
                i.coalesced,
                if i.build == crate::remote_daemon::build_id() {
                    ""
                } else {
                    " (another justrust build: replaced on next use)"
                }
            ),
            Err(_) => println!(
                "no build daemon running (starts with the next agent build; JUSTRUST_BUILDD=0 disables)"
            ),
        },
        "stop" => match info_request(b'S') {
            Ok(i) => println!("stopping build daemon pid {}", i.pid),
            Err(_) => println!("no build daemon running"),
        },
        "start" => {
            connect()?;
            println!("build daemon running");
        }
        "prebuild" => {
            let args: Vec<OsString> = args.into_iter().skip_while(|a| a == "--").collect();
            let req = request_for(&args, true)
                .context("prebuild takes `check|test|build|clippy ARGS...`")?;
            let mut s = connect()?;
            write_msg(&mut s, b'R', &serde_json::to_vec(&req)?)?;
            match read_msg(&mut s)? {
                Some((b'I', p)) => {
                    let i: Info = serde_json::from_slice(&p)?;
                    println!(
                        "{}",
                        if i.attached {
                            "an identical build is already running"
                        } else {
                            "prebuild started"
                        }
                    );
                }
                Some((b'F', p)) => bail!("{}", String::from_utf8_lossy(&p)),
                _ => bail!("bad answer from the build daemon"),
            }
        }
        _ => bail!("usage: justrust daemon status|start|stop|prebuild -- ARGS..."),
    }
    Ok(())
}

// ------------------------------------------------------------- source stamp

/// Directories whose contents can change what a build in `cwd` produces:
/// the workspace root and every path dependency outside it, found by
/// following `path = "..."` in manifests (no cargo metadata: 1.7 s on
/// Jcode Desktop).
pub fn source_roots(cwd: &Path) -> Vec<PathBuf> {
    let root = crate::early::workspace_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let mut roots = vec![root.clone()];
    let mut manifests: Vec<PathBuf> = vec![root.join("Cargo.toml")];
    let mut seen = std::collections::HashSet::new();
    let re = regex::Regex::new(r#"path\s*=\s*"([^"]+)""#).expect("regex");
    while let Some(m) = manifests.pop() {
        if !seen.insert(m.clone()) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&m) else {
            continue;
        };
        let base = m.parent().unwrap_or(Path::new("/"));
        for c in re.captures_iter(&text) {
            let p = normalize(&base.join(&c[1]));
            // `path` of a [lib]/[[bin]] target names a file, not a crate.
            let dir = if p.join("Cargo.toml").is_file() {
                p
            } else {
                continue;
            };
            manifests.push(dir.join("Cargo.toml"));
            if !roots.iter().any(|r| dir.starts_with(r)) {
                roots.push(dir);
            }
        }
        // Members listed by glob or name live under the root: covered.
    }
    roots.sort();
    roots.dedup_by(|b, a| b.starts_with(&*a));
    roots
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            c => out.push(c),
        }
    }
    out
}

/// Every package directory a build in `cwd` can compile from local source:
/// the workspace root, its members, and every path dependency reachable from
/// their manifests or named by a cargo config file (`[patch]`, `paths`).
/// `None` when a manifest, a config file or a member pattern cannot be read
/// with certainty, so that a caller pruning by this list falls back to pruning
/// nothing.
fn local_packages(cwd: &Path, configs: &[PathBuf]) -> Option<Vec<PathBuf>> {
    fn paths(v: &toml::Value, out: &mut Vec<String>) {
        match v {
            toml::Value::Table(t) => {
                for (k, v) in t {
                    match v {
                        toml::Value::String(s) if k == "path" => out.push(s.clone()),
                        _ => paths(v, out),
                    }
                }
            }
            toml::Value::Array(a) => a.iter().for_each(|v| paths(v, out)),
            _ => {}
        }
    }
    let root = crate::early::workspace_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let mut dirs = vec![root.clone()];
    let mut pending = vec![root];
    for config in configs {
        let Ok(text) = std::fs::read_to_string(config) else {
            continue;
        };
        let table: toml::Table = text.parse().ok()?;
        // Relative to the directory holding `.cargo`.
        let base = config.parent().and_then(Path::parent)?;
        let mut named = Vec::new();
        if let Some(overrides) = table.get("paths").and_then(|p| p.as_array()) {
            named.extend(
                overrides
                    .iter()
                    .filter_map(|p| p.as_str().map(String::from)),
            );
        }
        paths(&toml::Value::Table(table), &mut named);
        for p in named.iter().map(|p| normalize(&base.join(p))) {
            if p.join("Cargo.toml").is_file() {
                dirs.push(p.clone());
                pending.push(p);
            }
        }
    }
    let mut seen = std::collections::HashSet::new();
    while let Some(dir) = pending.pop() {
        if !seen.insert(dir.clone()) {
            continue;
        }
        let text = std::fs::read_to_string(dir.join("Cargo.toml")).ok()?;
        let manifest: toml::Table = text.parse().ok()?;
        let mut found = Vec::new();
        if let Some(members) = manifest
            .get("workspace")
            .and_then(|w| w.get("members"))
            .and_then(|m| m.as_array())
        {
            for member in members {
                found.extend(expand_member(&dir, member.as_str()?)?);
            }
        }
        let mut named = Vec::new();
        paths(&toml::Value::Table(manifest), &mut named);
        // `path` of a [lib]/[[bin]] target names a file, not a crate.
        found.extend(named.iter().map(|p| normalize(&dir.join(p))));
        for p in found {
            if p.join("Cargo.toml").is_file() {
                dirs.push(p.clone());
                pending.push(p);
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    Some(dirs)
}

/// The directories a `[workspace] members` entry names. Literal components
/// and `*`/`?` within one component are expanded; anything else (`**`,
/// character classes) is `None`: not understood, so nothing may be pruned.
fn expand_member(base: &Path, pattern: &str) -> Option<Vec<PathBuf>> {
    let mut dirs = vec![base.to_path_buf()];
    for part in Path::new(pattern).components() {
        let std::path::Component::Normal(part) = part else {
            dirs.iter_mut().for_each(|d| d.push(part));
            continue;
        };
        let part = part.to_str()?;
        if part.contains(['[', ']', '{', '}']) || part.contains("**") {
            return None;
        }
        if !part.contains(['*', '?']) {
            dirs.iter_mut().for_each(|d| d.push(part));
            continue;
        }
        let mut next = Vec::new();
        for d in &dirs {
            let Ok(rd) = std::fs::read_dir(d) else {
                continue;
            };
            for e in rd.flatten() {
                if e.file_name().to_str().is_some_and(|n| wildcard(part, n)) {
                    next.push(e.path());
                }
            }
        }
        dirs = next;
    }
    Some(dirs.iter().map(|d| normalize(d)).collect())
}

/// `*` and `?` within one path component.
fn wildcard(pattern: &str, name: &str) -> bool {
    let (p, n) = (pattern.as_bytes(), name.as_bytes());
    let (mut pi, mut ni) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ni < n.len() {
        if pi < p.len() && (p[pi] == b'?' || p[pi] == n[ni]) {
            pi += 1;
            ni += 1;
        } else if pi < p.len() && p[pi] == b'*' {
            star = Some((pi, ni));
            pi += 1;
        } else if let Some((sp, sn)) = star {
            star = Some((sp, sn + 1));
            pi = sp + 1;
            ni = sn + 1;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|b| *b == b'*')
}

/// What the stamp leaves out under one root.
///
/// Always: build output and hidden directories (except `.cargo`, whose
/// config changes builds).
///
/// In a source mirror on a build machine, also what the mirror's
/// `.justrustignore` keeps out of it. The sync never writes those paths, so
/// whatever an older mirror left there cannot change between two builds, yet
/// here they were 93,025 of the 94,450 files stamped before every build of
/// this repository (`/toolchain/`, `/bench/results/`), 150-320 ms warm and 2 s
/// once the kernel had dropped their inodes. A mirror is recognised by the
/// sync's own checkpoint token, which only a build machine's agent writes: a
/// local checkout has none and is stamped in full, because a local build can
/// read an edit anywhere in it.
///
/// The one local checkout that is stamped like a mirror is on a machine where
/// local compiles are off (`justrust local off`): every build of it runs on a
/// build machine, from a mirror that leaves the same paths out, so an edit
/// there cannot reach any build. Here that was 380,000 of 385,000 files and
/// 1.5 s before every build of this repository (104 requests, 184 s), and an
/// edit under `toolchain/` stopped and restarted the client's prebuild.
///
/// A package of this build inside an excluded tree is mirrored as a root of
/// its own (`remote_build::roots_of`), so the build does see edits to it:
/// those directories (`MirrorIgnore::packages`), and the way down to them,
/// are never pruned.
struct Prune {
    mirror: Option<MirrorIgnore>,
}

struct MirrorIgnore {
    root: PathBuf,
    rules: crate::remote_sync::JustrustIgnore,
    packages: Vec<PathBuf>,
}

impl Prune {
    fn of(root: &Path, cwd: &Path, configs: &[PathBuf], hosted_only: bool) -> Self {
        Self {
            mirror: MirrorIgnore::of(root, cwd, configs, hosted_only),
        }
    }

    fn hidden_or_output(name: &str) -> bool {
        name == "target"
            || name.starts_with("target-")
            || (name.starts_with('.') && name != ".cargo")
    }

    /// Whether the stamp leaves out `path`, an entry called `name`.
    fn skips(&self, path: &Path, name: &str, is_dir: bool) -> bool {
        (is_dir && Self::hidden_or_output(name))
            || self.mirror.as_ref().is_some_and(|m| m.skips(path, is_dir))
    }
}

impl MirrorIgnore {
    fn of(root: &Path, cwd: &Path, configs: &[PathBuf], hosted_only: bool) -> Option<Self> {
        #[cfg(test)]
        if tests::STAMP_EVERYTHING.with(|on| on.get()) {
            return None;
        }
        // The nearest mirror root at or above: a workspace below its
        // repository's root is mirrored with the repository. Where every
        // build is hosted, the checkout's repository is what gets mirrored
        // (`remote_build::roots_of`); outside git the whole workspace is,
        // and nothing is left out.
        let mirror = root
            .ancestors()
            .find(|a| a.join(crate::remote_checkpoint::TOKEN_FILE).is_file())
            .or_else(|| {
                hosted_only
                    .then(|| root.ancestors().find(|a| a.join(".git").exists()))
                    .flatten()
            })?;
        if !mirror.join(".justrustignore").is_file() {
            return None;
        }
        let rules = crate::remote_sync::JustrustIgnore::load(mirror);
        // Only a package the rules exclude is a root of its own. The others
        // are mirrored with this root, under its rules.
        let mut packages = local_packages(cwd, configs)?;
        packages.retain(|p| p.strip_prefix(mirror).is_ok_and(|rel| rules.matches(rel)));
        Some(Self {
            root: mirror.to_path_buf(),
            rules,
            packages,
        })
    }

    fn skips(&self, path: &Path, is_dir: bool) -> bool {
        let Ok(rel) = path.strip_prefix(&self.root) else {
            return false;
        };
        self.rules.matches(rel)
            && !self
                .packages
                .iter()
                .any(|p| path.starts_with(p) || (is_dir && p.starts_with(path)))
    }
}

/// Whether no build of this request's sources can compile on this machine.
fn hosted_only(env: &[(String, String)]) -> bool {
    #[cfg(test)]
    if tests::LOCAL_OFF.with(|on| on.get()) {
        return true;
    }
    crate::nolocal::disabled_in(env)
}

/// Hash of the source state under `roots`: every file's path, size, mtime
/// and inode that `Prune` does not leave out, plus cargo config files in the
/// ancestors of `cwd` and in CARGO_HOME.
pub fn source_stamp(roots: &[PathBuf], cwd: &Path, env: &[(String, String)]) -> (String, usize) {
    source_stamp_observed(roots, cwd, env, |_| {})
}

// The empty production observer is optimized away. The ignored diagnostic
// below measures the actual traversal without changing its inputs or policy.
fn source_stamp_observed(
    roots: &[PathBuf],
    cwd: &Path,
    env: &[(String, String)],
    phase: impl FnMut(&'static str),
) -> (String, usize) {
    let mut phase = phase;
    stamp_walk(roots, cwd, env, |root, prune, add| {
        phase("root-start");
        // Sorted siblings visited depth first produce Path's component order.
        let mut stack = vec![(root.to_owned(), true)];
        while let Some((path, directory)) = stack.pop() {
            if !directory {
                if let Ok(m) = std::fs::metadata(&path) {
                    add(&path, &m);
                }
                continue;
            }
            let Ok(rd) = std::fs::read_dir(&path) else {
                continue;
            };
            let first = stack.len();
            for entry in rd.flatten() {
                let Ok(ft) = entry.file_type() else { continue };
                let path = entry.path();
                if (ft.is_dir() || prune.mirror.is_some())
                    && prune.skips(&path, &entry.file_name().to_string_lossy(), ft.is_dir())
                {
                    continue;
                }
                stack.push((path, ft.is_dir()));
            }
            stack[first..].sort_unstable_by(|a, b| b.0.as_os_str().cmp(a.0.as_os_str()));
        }
        phase("walk-hash");
    })
}

#[cfg(test)]
fn source_stamp_with_sort(
    roots: &[PathBuf],
    cwd: &Path,
    env: &[(String, String)],
    mut phase: impl FnMut(&'static str),
    mut sort: impl FnMut(&mut [(PathBuf, std::fs::Metadata)]),
) -> (String, usize) {
    stamp_walk(roots, cwd, env, |r, prune, add| {
        phase("root-start");
        let mut stack = vec![r.to_owned()];
        let mut entries: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name();
                let n = name.to_string_lossy();
                let Ok(ft) = e.file_type() else { continue };
                if prune.skips(&e.path(), &n, ft.is_dir()) {
                    continue;
                }
                if ft.is_dir() {
                    stack.push(e.path());
                } else if let Ok(m) = std::fs::metadata(e.path()) {
                    entries.push((e.path(), m));
                }
            }
        }
        phase("enumerate-metadata");
        sort(&mut entries);
        phase("sort");
        for (p, m) in &entries {
            add(p, m);
        }
        phase("hash");
        drop(entries);
        phase("drop-entries");
    })
}

fn stamp_walk(
    roots: &[PathBuf],
    cwd: &Path,
    env: &[(String, String)],
    mut walk: impl FnMut(&Path, &Prune, &mut dyn FnMut(&Path, &std::fs::Metadata)),
) -> (String, usize) {
    let mut h = crate::depcache::H128::new();
    let mut files = 0usize;
    let mut add = |p: &Path, m: &std::fs::Metadata| {
        h.field(p.as_os_str().as_encoded_bytes());
        h.field(&m.len().to_le_bytes());
        let (s, ns) = crate::sys::mtime(m);
        h.field(&s.to_le_bytes());
        h.field(&ns.to_le_bytes());
        h.field(&crate::sys::file_id(m).1.to_le_bytes());
    };
    let cargo_home = env
        .iter()
        .find(|(k, _)| k == "CARGO_HOME")
        .map(|(_, v)| PathBuf::from(v))
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")));
    let mut configs: Vec<PathBuf> = cwd
        .ancestors()
        .flat_map(|a| [a.join(".cargo/config.toml"), a.join(".cargo/config")])
        .collect();
    if let Some(ch) = cargo_home {
        configs.push(ch.join("config.toml"));
        configs.push(ch.join("config"));
    }
    let hosted_only = hosted_only(env);
    for r in roots {
        walk(r, &Prune::of(r, cwd, &configs, hosted_only), &mut |p, m| {
            files += 1;
            add(p, m);
        });
    }
    for c in configs {
        if let Ok(m) = std::fs::metadata(&c) {
            add(&c, &m);
        }
    }
    (h.hex(), files)
}

/// These paths come from readdir under one unchanged root spelling. Their
/// common prefix may contain `.`/`..` or repeated separators, but every suffix
/// component is a real directory entry, never `.` or `..`. On Unix, replacing
/// separators with NUL gives exactly Path's component order: a component end
/// sorts before every filename byte. NUL cannot occur in a filesystem name.
///
/// Reuse the owned path buffers, sort raw keys, then restore every byte before
/// hashing. No metadata is reused between traversals and no path is omitted.
#[cfg(test)]
fn sort_stamp_root_entries<T>(entries: &mut [(PathBuf, T)]) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        let replace = |path: &mut PathBuf, from, to| {
            let mut bytes = std::mem::take(path).into_os_string().into_vec();
            for byte in &mut bytes {
                if *byte == from {
                    *byte = to;
                }
            }
            *path = std::ffi::OsString::from_vec(bytes).into();
        };
        for (path, _) in entries.iter_mut() {
            replace(path, b'/', 0);
        }
        entries.sort_by(|a, b| a.0.as_os_str().as_bytes().cmp(b.0.as_os_str().as_bytes()));
        for (path, _) in entries.iter_mut() {
            replace(path, 0, b'/');
        }
    }
    #[cfg(not(unix))]
    entries.sort_by(|a, b| a.0.cmp(&b.0));
}

/// Request identity without the source stamp: cwd, argv, and the
/// environment minus [`IGNORED_ENV`].
pub fn request_key(req: &Request) -> String {
    let mut h = crate::depcache::H128::new();
    h.field(req.cwd.as_bytes());
    for a in &req.argv {
        h.field(a.as_bytes());
    }
    h.field(b"--env--");
    for (k, v) in req.env.iter().filter(|(k, _)| !env_ignored(k)) {
        h.field(k.as_bytes());
        h.field(v.as_bytes());
    }
    h.hex()
}

// ------------------------------------------------------------------ daemon

/// One build's output so far and its outcome.
#[derive(Default)]
struct Log {
    chunks: Vec<(u8, Vec<u8>)>,
    exit: Option<i32>,
    clients: usize,
    sharers: usize,
    /// An agent request attached at some point (its result was seen).
    claimed: bool,
    /// Index in `chunks` right after the early-exit marker
    /// ([`crate::early::EXIT_MARKER`]), when the build printed one.
    early_at: Option<usize>,
    /// A client already took the early answer: later clients wait for
    /// rustc, and nobody is left to stop the build when it is gone.
    early_served: bool,
}

struct Build {
    /// None means native Cargo must validate freshness. Never authorize reuse.
    stamp: Option<String>,
    /// A `test` prebuilt as `test --no-run`: requests wait for it, then run
    /// for real (on Fresh units), never take its output.
    compile_only: Mutex<bool>,
    /// What a compile-only prebuild compiles ([`units_key`]): a request for
    /// other tests of the same units waits for it too.
    units_key: Mutex<Option<String>>,
    /// Agent session that asked (`JUSTRUST_SLOT_SESSION`).
    session: String,
    /// Workspace directory (`req.cwd`).
    cwd: String,
    run_id: Option<String>,
    started: Instant,
    finished: Mutex<Option<Instant>>,
    pid: i32,
    prebuild: Mutex<bool>,
    /// The daemon terminated it ([`stop`]): its exit is not a result.
    stopped: std::sync::atomic::AtomicBool,
    /// A prebuild's [`prebuild_claim`], held until it exits.
    claim: Mutex<Option<std::fs::File>>,
    log: Mutex<Log>,
    cv: Condvar,
}

#[derive(Default)]
struct State {
    /// In-flight builds by request key.
    builds: HashMap<String, Arc<Build>>,
    /// Prebuilds that finished before anyone asked, by request key. The
    /// first identical request with the same source stamp takes the result
    /// (once: a deliberate rerun after that runs again).
    finished: HashMap<String, Arc<Build>>,
    /// Requests to prebuild when their sources change, by request key.
    recent: HashMap<String, Recent>,
    /// Per-workspace (`cwd`) adaptive backoff after wasted/cancelled prebuilds.
    prebuild_backoff: HashMap<String, PrebuildBackoff>,
    served: u64,
    coalesced: u64,
    prebuilt: u64,
    last_request: Option<Instant>,
}

#[derive(Default)]
struct PrebuildBackoff {
    misses: u32,
    cooldown_until: Option<Instant>,
}

/// A request to prebuild when its sources change (`JUSTRUST_BUILDD_PREBUILD=0` disables).
struct Recent {
    req: Request,
    roots: Vec<PathBuf>,
    used: Instant,
    /// Number of real requests seen for this key. Prebuilds only start once
    /// an edit-build loop is established (`seen >= MIN_REQUESTS_TO_PREBUILD`).
    seen: u32,
    /// Prebuilds left until the agent asks again. Bounds the loop of a
    /// build that writes into its own sources (snapshot tests, generated
    /// files): each of its writes would otherwise start another prebuild.
    budget: u8,
}

const PREBUILDS_PER_REQUEST: u8 = 3;
/// Require at least two real requests for the same command before firing
/// speculative prebuilds, so one-off commands don't waste build slots.
const MIN_REQUESTS_TO_PREBUILD: u32 = 2;

/// Prebuild only what was asked for lately: each agent session's last
/// request (right for 53% of 3,436 mined post-edit build calls, 0.47
/// wasted prebuilds per edit; the last 3 were right 68% but wasted 1.59).
const RECENT_MAX_AGE: Duration = Duration::from_secs(1800);
const RECENT_MAX: usize = 8;
/// Quiet time after the last file event before prebuilding: an agent's
/// multi-file edit lands as one burst.
const DEBOUNCE: Duration = Duration::from_millis(400);
/// An unclaimed finished prebuild is dropped after this long.
const FINISHED_MAX_AGE: Duration = Duration::from_secs(1800);

struct Daemon {
    state: Mutex<State>,
    /// File watcher for prebuilds, and the roots it covers.
    watcher: Option<crate::remote_watch::Watcher>,
    watched: Mutex<std::collections::HashSet<PathBuf>>,
    /// (path, dev, ino) of our socket: removed on exit only if still ours.
    sock: (PathBuf, (u64, u64)),
    /// (path, dev, ino) of our executable at startup: detect atomic replacement.
    exe: Option<(PathBuf, (u64, u64))>,
    draining: std::sync::atomic::AtomicBool,
    /// Prebuilds after a toolchain change or a daemon replacement.
    warm: warm::Warm,
}

impl Daemon {
    fn exe_replaced(&self) -> bool {
        let cur = std::env::current_exe().ok();
        exe_is_replaced(self.exe.as_ref(), cur.as_deref())
    }
}

fn drain_and_exit(dm: &Arc<Daemon>, reason: &str) {
    if dm.draining.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    remove_sock(dm);
    log(reason);
    let dm = dm.clone();
    std::thread::spawn(move || {
        while !dm.state.lock().unwrap().builds.is_empty() {
            std::thread::sleep(Duration::from_millis(100));
        }
        std::process::exit(0);
    });
}

fn log(msg: &str) {
    eprintln!(
        "[{}] [{}] {msg}",
        chrono::Local::now().format("%F %T%.3f"),
        std::process::id()
    );
}

pub fn daemon_main() -> ! {
    let code = match daemon() {
        Ok(()) => 0,
        Err(e) => {
            log(&format!("build daemon: {e:#}"));
            1
        }
    };
    std::process::exit(code)
}

fn daemon() -> Result<()> {
    let d = dir()?;
    std::fs::create_dir_all(&d)?;
    let lock_path = lock_path()?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)?;
    if lock.try_lock().is_err() {
        bail!("another build daemon is running");
    }
    // A sweeper may have removed the file between the open and the lock.
    let same_file = |a: std::io::Result<std::fs::Metadata>,
                     b: std::io::Result<std::fs::Metadata>| {
        matches!((a, b), (Ok(a), Ok(b)) if crate::sys::file_id(&a) == crate::sys::file_id(&b))
    };
    if !same_file(lock.metadata(), std::fs::metadata(&lock_path)) {
        bail!("the build daemon lock was replaced while starting");
    }
    // Its mtime says when a daemon of this build last started.
    let _ = (&lock).write_all(b"\n");
    let _ = lock.set_len(0);
    sweep_stale(&d, &lock_path, STALE_BUILD_FILES);
    let sock = sock_path()?;
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    let md = std::fs::metadata(&sock)?;
    log(&format!(
        "build daemon up, build {}",
        crate::remote_daemon::build_id()
    ));
    let (wtx, wrx) = std::sync::mpsc::channel::<PathBuf>();
    let warm = warm::Warm::new(wtx.clone());
    let watcher = crate::remote_watch::Watcher::new(wtx)
        .map_err(|e| log(&format!("no file watcher, prebuilds off: {e:#}")))
        .ok();
    let dm = Arc::new(Daemon {
        state: Mutex::new(State {
            last_request: Some(Instant::now()),
            ..Default::default()
        }),
        watcher,
        watched: Mutex::default(),
        sock: (sock, crate::sys::file_id(&md)),
        exe: current_exe_identity(),
        draining: Default::default(),
        warm,
    });
    {
        let dm = dm.clone();
        std::thread::spawn(move || warm::run(&dm));
    }
    {
        let dm = dm.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(30));
                let st = dm.state.lock().unwrap();
                let idle =
                    st.builds.is_empty() && st.last_request.is_none_or(|t| t.elapsed() > IDLE_EXIT);
                drop(st);
                if idle {
                    log("idle, exiting");
                    remove_sock(&dm);
                    std::process::exit(0);
                }
            }
        });
    }
    {
        let dm = dm.clone();
        std::thread::spawn(move || prebuild_loop(&dm, wrx));
    }
    for conn in listener.incoming() {
        let Ok(s) = conn else { continue };
        if dm.draining.load(std::sync::atomic::Ordering::SeqCst) {
            let mut s = s;
            let _ = write_msg(&mut s, b'F', b"build daemon is shutting down");
            continue;
        }
        let dm = dm.clone();
        std::thread::spawn(move || {
            if let Err(e) = serve(&dm, s) {
                log(&format!("client: {e:#}"));
            }
        });
    }
    Ok(())
}

fn remove_sock(dm: &Daemon) {
    let (p, id) = &dm.sock;
    if std::fs::metadata(p).is_ok_and(|m| crate::sys::file_id(&m) == *id) {
        let _ = std::fs::remove_file(p);
    }
}

fn info(dm: &Daemon, attached: bool) -> Info {
    let st = dm.state.lock().unwrap();
    Info {
        build: crate::remote_daemon::build_id(),
        pid: std::process::id(),
        running: st.builds.len(),
        served: st.served,
        coalesced: st.coalesced + st.prebuilt,
        attached,
    }
}

fn serve(dm: &Arc<Daemon>, mut s: UnixStream) -> Result<()> {
    let Some((kind, body)) = read_msg(&mut s)? else {
        return Ok(());
    };
    match kind {
        b'P' => {
            write_msg(&mut s, b'I', &serde_json::to_vec(&info(dm, false))?)?;
            Ok(())
        }
        b'S' => {
            write_msg(&mut s, b'I', &serde_json::to_vec(&info(dm, false))?)?;
            drain_and_exit(dm, "shutdown requested, draining");
            Ok(())
        }
        b'R' => {
            let req: Request = serde_json::from_slice(&body)?;
            if req.build != crate::remote_daemon::build_id() && !req.build.is_empty() {
                // Another justrust build: hand back and step aside, so the
                // client's next request starts a daemon of its own build.
                write_msg(&mut s, b'F', b"build daemon is another justrust build")?;
                drain_and_exit(dm, "replaced by another justrust build, draining");
                return Ok(());
            }
            if dm.exe_replaced() {
                // The binary on disk was replaced (e.g. atomic upgrade) while
                // this daemon was running: step aside so the client starts a
                // fresh daemon instead of this one trying to exec `(deleted)`.
                write_msg(&mut s, b'F', b"build daemon binary was replaced on disk")?;
                drain_and_exit(dm, "binary replaced on disk, draining");
                return Ok(());
            }
            run(dm, req, s)
        }
        _ => bail!("unknown request {kind}"),
    }
}

fn run(dm: &Arc<Daemon>, req: Request, mut s: UnixStream) -> Result<()> {
    let t0 = Instant::now();
    let cwd = PathBuf::from(&req.cwd);
    let key = request_key(&req);
    let roots = source_roots(&cwd);
    // Eligibility and reservation share the lock with both normal requests and
    // the watcher. No scan result, synthetic stamp, or cached Cargo verdict is
    // involved: the child must run Cargo's complete freshness path.
    let native = {
        let mut st = dm.state.lock().unwrap();
        if native_empty_eligible(&st, &req) || native_no_candidate_eligible(&st, &req, &key) {
            let b = start(&req, None, &cwd)?;
            st.served += 1;
            st.last_request = Some(Instant::now());
            remember_request(&mut st, &req, &key, &roots);
            st.builds.insert(key.clone(), b.clone());
            let (dm2, key2, b2) = (dm.clone(), key.clone(), b.clone());
            std::thread::spawn(move || reap(&dm2, &key2, &b2));
            Some(b)
        } else {
            None
        }
    };
    if let Some(build) = native {
        return serve_build(
            dm,
            &req,
            &roots,
            &mut s,
            build,
            false,
            false,
            t0,
            (0.0, 0),
            0.0,
        );
    }
    let (stamp, files) = source_stamp(&roots, &cwd, &req.env);
    let scan = (t0.elapsed().as_secs_f64() * 1000.0, files);
    let wait_start = Instant::now();
    // The compile-only prebuild whose units this request reuses, and how
    // long the request waited for it.
    let mut waited_for: Option<(Arc<Build>, f64)> = None;
    if !req.prebuild {
        cancel_stale_prebuild(dm, &key, &stamp);
        // Before the wrong guesses are stopped: this one is not wrong.
        let same_units = claim_same_units(dm, &req, &key, &stamp);
        cancel_wrong_guess(dm, &key, &session_of(&req));
        await_stopped(dm, &key);
        let t = Instant::now();
        if let Some(b) = settle_compile_only(dm, &key, &stamp) {
            waited_for = Some((b, t.elapsed().as_secs_f64()));
        }
        if let Some(b) = same_units {
            log(&format!(
                "waiting for compile-only prebuild pid {} of the same units",
                b.pid
            ));
            let t = Instant::now();
            let mut l = b.log.lock().unwrap();
            while l.exit.is_none() {
                l = b.cv.wait(l).unwrap();
            }
            drop(l);
            waited_for = Some((b, t.elapsed().as_secs_f64()));
        }
    }
    // A prebuild that was stopped or killed left nothing to report.
    waited_for.take_if(|(b, _)| no_result(b, &b.log.lock().unwrap()));
    let waited_ms = wait_start.elapsed().as_secs_f64() * 1000.0;
    let (build, attached, replayed) = {
        let mut st = dm.state.lock().unwrap();
        // A native request can have arrived while this request scanned. Never
        // attach to an unstamped build or overwrite its active reservation.
        if native_key_occupied(&st, &key) {
            write_msg(
                &mut s,
                b'F',
                b"native freshness build already owns this request",
            )?;
            return Ok(());
        }
        st.served += 1;
        st.last_request = Some(Instant::now());
        let running = st
            .builds
            .get(&key)
            .filter(|b| {
                b.stamp.as_deref() == Some(&*stamp)
                    && !stopped(b)
                    && b.log.lock().unwrap().exit.is_none()
                    && !*b.compile_only.lock().unwrap()
            })
            .cloned();
        // A prebuild of exactly this request finished before it arrived.
        let done = if req.prebuild || running.is_some() {
            None
        } else {
            st.finished.remove(&key).filter(|b| {
                b.stamp.as_deref() == Some(&*stamp)
                    && b.started.elapsed() < FINISHED_MAX_AGE
                    && !*b.compile_only.lock().unwrap()
                    && !no_result(b, &b.log.lock().unwrap())
            })
        };
        remember_request(&mut st, &req, &key, &roots);
        if done.is_some() {
            st.prebuilt += 1;
        }
        let replayed = done.is_some();
        match running.or(done) {
            Some(b) => {
                if !replayed {
                    st.coalesced += 1;
                }
                (b, true, replayed)
            }
            _ => {
                // The run that follows a prebuild of its units finds them
                // Fresh: tell it whose work that was.
                let waited = waited_for
                    .as_ref()
                    .and_then(|(b, secs)| waited_env(b.run_id.as_deref(), *secs));
                let mut told = req.clone();
                told.env.retain(|(k, _)| k != WAITED_ENV);
                told.env.extend(waited.map(|v| (WAITED_ENV.to_owned(), v)));
                let b = start(&told, Some(stamp), &cwd)?;
                st.builds.insert(key.clone(), b.clone());
                let (dm2, key2, b2) = (dm.clone(), key.clone(), b.clone());
                std::thread::spawn(move || reap(&dm2, &key2, &b2));
                (b, false, false)
            }
        }
    };
    serve_build(
        dm, &req, &roots, &mut s, build, attached, replayed, t0, scan, waited_ms,
    )
}

fn native_empty_eligible(st: &State, req: &Request) -> bool {
    // source_roots is a watch/stamp heuristic, not a dependency or isolation
    // proof. Disjoint roots also do not exempt same-session wrong guesses
    // from cancellation. Keep the global exclusion until coordination has
    // an authority stronger than those paths.
    !req.prebuild
        && req
            .env
            .iter()
            .any(|(k, v)| k == "JUSTRUST_BUILDD_NATIVE_EMPTY" && v == "1")
        && st.builds.is_empty()
        && st.finished.is_empty()
}

fn native_key_occupied(st: &State, key: &str) -> bool {
    st.builds.get(key).is_some_and(|b| b.stamp.is_none())
}

/// Skip only operations which have no candidate at this locked reservation.
/// This makes no claim about path or dependency isolation: Cargo still runs.
fn native_no_candidate_eligible(st: &State, req: &Request, key: &str) -> bool {
    !req.prebuild
        && req.env.iter().find(|(k, _)| k == "JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE").map_or_else(
            || matches!(req.argv.first().map(String::as_str), Some("check" | "c" | "clippy")),
            |(_, v)| v == "1",
        )
        // Covers stale cancellation, compile-only settlement, attach, finished
        // removal/replay and unstamped same-key reservations, without a stamp.
        && !st.builds.contains_key(key)
        && !st.finished.contains_key(key)
        && !st
            .builds
            .iter()
            .any(|(k, b)| wrong_guess_candidate(k, b, key, &session_of(req)))
}

fn wrong_guess_candidate(build_key: &str, b: &Build, key: &str, session: &str) -> bool {
    if build_key == key || b.session != session {
        return false;
    }
    let l = b.log.lock().unwrap();
    l.exit.is_none() && ((*b.prebuild.lock().unwrap() && !l.claimed) || l.early_served)
}

fn remember_request(st: &mut State, req: &Request, key: &str, roots: &[PathBuf]) {
    if !req.prebuild && wants_prebuild(req) {
        let seen = st.recent.get(key).map_or(1, |r| r.seen.saturating_add(1));
        st.recent.insert(
            key.to_owned(),
            Recent {
                req: req.clone(),
                roots: roots.to_vec(),
                used: Instant::now(),
                seen,
                budget: PREBUILDS_PER_REQUEST,
            },
        );
        prune_recent(&mut st.recent);
    }
}

#[allow(clippy::too_many_arguments)]
fn serve_build(
    dm: &Arc<Daemon>,
    req: &Request,
    roots: &[PathBuf],
    s: &mut UnixStream,
    build: Arc<Build>,
    attached: bool,
    replayed: bool,
    t0: Instant,
    (scan_ms, scan_files): (f64, usize),
    waited_ms: f64,
) -> Result<()> {
    let lookup_ms = t0.elapsed().as_secs_f64() * 1000.0;
    if !req.prebuild && wants_prebuild(req) {
        watch_roots(dm, roots);
    }
    let joined_after = match *build.finished.lock().unwrap() {
        // Negative: seconds since the replayed prebuild finished.
        Some(t) if replayed => -t.elapsed().as_secs_f64(),
        _ if attached => build.started.elapsed().as_secs_f64(),
        _ => 0.0,
    };
    {
        let mut l = build.log.lock().unwrap();
        l.sharers += 1;
        if !req.prebuild {
            l.clients += 1;
            l.claimed = true;
        }
    }
    if req.prebuild {
        *build.prebuild.lock().unwrap() = true;
        write_msg(s, b'I', &serde_json::to_vec(&info(dm, attached))?)?;
        return Ok(());
    }
    log(&format!(
        "{} {} in {} ({:.1} ms lookup, {:.1} ms source validation of {} files, {:.1} ms prebuild coordination, stamp {})",
        if replayed {
            "replay"
        } else if attached {
            "attach"
        } else {
            "start"
        },
        req.argv.join(" "),
        req.cwd,
        lookup_ms,
        scan_ms,
        scan_files,
        waited_ms,
        build
            .stamp
            .as_deref()
            .map(|s| s.get(..8).unwrap_or(s))
            .unwrap_or("native")
    ));
    let res = stream(&build, s, attached, replayed, joined_after, lookup_ms);
    let mut l = build.log.lock().unwrap();
    l.clients -= 1;
    let orphaned = stop_when_orphaned(&l, *build.prebuild.lock().unwrap());
    drop(l);
    if orphaned {
        // Everyone who asked is gone (killed or timed out): stop the build
        // like their Ctrl+C or kill would have without the daemon.
        log(&format!("no clients left, stopping pid {}", build.pid));
        stop(&build);
    }
    res
}

/// The last client left: stop the build, unless it already answered early
/// (the point of buildd: rustc finishes so the verdict replays and the cache
/// fills) or it is a prebuild nobody claimed yet. A stopped build releases
/// its compiler usage lease when cargo exits; a kept one holds it until
/// cargo finishes (docs/toolchain-slots.md).
fn stop_when_orphaned(l: &Log, prebuild: bool) -> bool {
    l.clients == 0 && l.exit.is_none() && !l.early_served && !prebuild
}

fn stream(
    build: &Build,
    s: &mut UnixStream,
    attached: bool,
    replayed: bool,
    joined_after: f64,
    lookup_ms: f64,
) -> Result<()> {
    let mut next = 0usize;
    loop {
        let (batch, exit, sharers, early) = {
            let mut l = build.log.lock().unwrap();
            while next >= l.chunks.len()
                && l.exit.is_none()
                && !l.early_at.is_some_and(|e| e <= next && !l.early_served)
            {
                // A client killed while the build is quiet is noticed
                // within a second, not at the next output.
                let (g, t) = build.cv.wait_timeout(l, Duration::from_secs(1)).unwrap();
                l = g;
                if t.timed_out() && peer_closed(s) {
                    bail!("client went away");
                }
            }
            // The first client to reach the marker of a running build gets
            // the early answer: output up to the marker, then exit 101.
            let early = l.exit.is_none() && !l.early_served && l.early_at.is_some();
            let upto = match l.early_at {
                Some(e) if early => e.max(next),
                _ => l.chunks.len(),
            };
            let batch: Vec<(u8, Vec<u8>)> = l.chunks[next..upto].to_vec();
            next = upto;
            let early = early && next >= l.early_at.unwrap_or(usize::MAX);
            if early {
                l.early_served = true;
            }
            (
                batch,
                if next >= l.chunks.len() { l.exit } else { None },
                l.sharers,
                early,
            )
        };
        for (k, b) in &batch {
            write_msg(s, *k, b)?;
        }
        if early {
            let x = Exit {
                code: 101,
                attached,
                joined_after,
                sharers,
                lookup_ms,
                run_id: build.run_id.clone(),
                early: true,
                ..Default::default()
            };
            write_msg(s, b'X', &serde_json::to_vec(&x)?)?;
            return Ok(());
        }
        if let Some(code) = exit {
            if dropped_guess(build, &build.log.lock().unwrap()) {
                // Nothing was printed: the client builds for itself.
                write_msg(s, b'F', b"the prebuild declined to run")?;
                return Ok(());
            }
            if stopped(build) {
                // Its exit code is the daemon's signal, not a verdict.
                write_msg(s, b'F', b"the build daemon stopped this build")?;
                return Ok(());
            }
            let x = Exit {
                code,
                attached,
                joined_after,
                sharers,
                lookup_ms,
                run_id: build.run_id.clone(),
                replayed,
                after_early: replayed && build.log.lock().unwrap().early_served,
                early: false,
            };
            write_msg(s, b'X', &serde_json::to_vec(&x)?)?;
            return Ok(());
        }
    }
}

/// The client closed its end (it never sends after the request).
#[cfg(unix)]
fn peer_closed(s: &UnixStream) -> bool {
    use std::os::fd::AsRawFd;
    let mut b = [0u8; 1];
    // SAFETY: recv into a local buffer on a socket we own.
    let n = unsafe {
        libc::recv(
            s.as_raw_fd(),
            b.as_mut_ptr().cast(),
            1,
            libc::MSG_PEEK | libc::MSG_DONTWAIT,
        )
    };
    n == 0
        || (n < 0
            && !matches!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EAGAIN) | Some(libc::EINTR)
            ))
}

/// Windows: a non-blocking peek; 0 bytes means the client closed.
#[cfg(windows)]
fn peer_closed(s: &UnixStream) -> bool {
    use std::os::windows::io::AsRawSocket;
    use windows_sys::Win32::Networking::WinSock::{
        MSG_PEEK, WSAEWOULDBLOCK, WSAGetLastError, recv,
    };
    if s.set_nonblocking(true).is_err() {
        return false;
    }
    let mut b = [0u8; 1];
    // SAFETY: recv into a local buffer on a socket we own.
    let n = unsafe { recv(s.as_raw_socket() as usize, b.as_mut_ptr(), 1, MSG_PEEK) };
    // SAFETY: no arguments.
    let err = (n < 0).then(|| unsafe { WSAGetLastError() });
    let _ = s.set_nonblocking(false);
    n == 0 || err.is_some_and(|e| e != WSAEWOULDBLOCK)
}

/// Another prebuild holds the claim.
#[derive(Debug, PartialEq)]
struct Busy;

/// The right to prebuild `req`'s command in its directory for its session
/// (sessions build in separate slots). A daemon runs per client build, so
/// several see the same edit, and requests that differ only in environment
/// have separate keys in one daemon: each started the same prebuild and all
/// but one waited for the build directory (252 of 270 doubled prebuilds on
/// the hosted machine came from two daemons). The lock dies with its holder.
/// Fails open: no claim file, no coordination.
fn prebuild_claim(dir: &Path, req: &Request) -> Result<Option<std::fs::File>, Busy> {
    let mut h = crate::depcache::H128::new();
    h.field(req.cwd.as_bytes());
    for a in &req.argv {
        h.field(a.as_bytes());
    }
    h.field(b"--session--");
    h.field(session_of(req).as_bytes());
    let hex = h.hex();
    let path = dir.join(format!("{}.claim", hex.get(..16).unwrap_or(&hex)));
    let _ = std::fs::create_dir_all(dir);
    let Ok(f) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
    else {
        return Ok(None);
    };
    match f.try_lock() {
        Ok(()) => Ok(Some(f)),
        Err(std::fs::TryLockError::WouldBlock) => Err(Busy),
        Err(std::fs::TryLockError::Error(_)) => Ok(None),
    }
}

/// A workspace that was deleted (a scratch checkout, a finished CI job):
/// deleting it wakes the watcher like an edit does, and a prebuild cannot
/// start in a directory that is gone. Nobody will ask for it again either.
fn forget_if_gone<V>(recent: &mut HashMap<String, V>, key: &str, cwd: &Path) -> bool {
    if cwd.is_dir() {
        return false;
    }
    if recent.remove(key).is_some() {
        log(&format!(
            "workspace {} is gone, no more prebuilds for it",
            cwd.display()
        ));
    }
    true
}

fn start(req: &Request, stamp: Option<String>, cwd: &Path) -> Result<Arc<Build>> {
    let mut c = Command::new(live_exe()?);
    c.args(&req.argv)
        .current_dir(cwd)
        .env_clear()
        .envs(req.env.iter().map(|(k, v)| (k, v)))
        .env(CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c.env_remove(STAMP_ENV);
    if let Some(s) = &stamp {
        c.env(STAMP_ENV, s);
    }
    c.detached();
    if req.prebuild {
        // A guess: below every build an agent is waiting for.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            let nice = warm::niceness(req);
            // SAFETY: setpriority is async-signal-safe.
            unsafe {
                c.pre_exec(move || {
                    libc::setpriority(libc::PRIO_PROCESS, 0, nice);
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(
                windows_sys::Win32::System::Threading::CREATE_NEW_PROCESS_GROUP
                    | windows_sys::Win32::System::Threading::CREATE_NO_WINDOW
                    | windows_sys::Win32::System::Threading::BELOW_NORMAL_PRIORITY_CLASS,
            );
        }
    }
    crate::procfs::close_inherited_fds(&mut c);
    let mut child = c.spawn().context("starting the build")?;
    let b = Arc::new(Build {
        stamp,
        compile_only: Mutex::default(),
        units_key: Mutex::default(),
        session: session_of(req),
        cwd: req.cwd.clone(),
        run_id: req
            .env
            .iter()
            .find(|(k, _)| k == "JUSTRUST_RUN_ID_OVERRIDE")
            .map(|(_, v)| v.clone()),
        started: Instant::now(),
        finished: Mutex::default(),
        pid: child.id() as i32,
        prebuild: Mutex::new(req.prebuild),
        stopped: Default::default(),
        claim: Mutex::default(),
        log: Mutex::default(),
        cv: Condvar::new(),
    });
    let out = child.stdout.take().context("stdout")?;
    let err = child.stderr.take().context("stderr")?;
    let pumps = [
        (b'O', Box::new(out) as Box<dyn Read + Send>),
        (b'E', Box::new(err)),
    ]
    .into_iter()
    .map(|(k, mut r)| {
        let b = b.clone();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 << 10];
            loop {
                match r.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        let mut l = b.log.lock().unwrap();
                        match crate::early::split_exit_marker(&buf[..n]) {
                            Some((before, after)) if k == b'E' => {
                                if !before.is_empty() {
                                    l.chunks.push((k, before.to_vec()));
                                }
                                if l.early_at.is_none() {
                                    l.early_at = Some(l.chunks.len());
                                }
                                if !after.is_empty() {
                                    l.chunks.push((k, after.to_vec()));
                                }
                            }
                            _ => l.chunks.push((k, buf[..n].to_vec())),
                        }
                        drop(l);
                        b.cv.notify_all();
                    }
                }
            }
        })
    })
    .collect::<Vec<_>>();
    // The waiter owns the child; `reap` waits for this thread's result.
    let b2 = b.clone();
    std::thread::spawn(move || {
        let status = child.wait();
        for p in pumps {
            let _ = p.join();
        }
        let code = match status {
            Ok(s) => crate::sys::exit_code(s),
            Err(_) => 1,
        };
        *b2.finished.lock().unwrap() = Some(Instant::now());
        b2.log.lock().unwrap().exit = Some(code);
        b2.cv.notify_all();
    });
    Ok(b)
}

/// A prebuild of this request for older sources, that no agent attached
/// to, is stopped (and awaited, so it releases its slot) before the real
/// request starts: it would only compete for the slot and the CPU.
fn cancel_stale_prebuild(dm: &Daemon, key: &str, stamp: &str) {
    let stale = {
        let st = dm.state.lock().unwrap();
        st.builds
            .get(key)
            .filter(|b| {
                let l = b.log.lock().unwrap();
                // A build that already answered early is as stale as an
                // unclaimed prebuild: nobody waits for it, and it holds the
                // target dir the new request needs.
                b.stamp.as_deref() != Some(&*stamp)
                    && l.exit.is_none()
                    && ((*b.prebuild.lock().unwrap() && !l.claimed) || l.early_served)
            })
            .cloned()
    };
    let Some(b) = stale else { return };
    log(&format!(
        "sources changed, stopping {} pid {}",
        if b.log.lock().unwrap().early_served {
            "build that answered early,"
        } else {
            "prebuild"
        },
        b.pid
    ));
    stop(&b);
    let mut l = b.log.lock().unwrap();
    let t = Instant::now();
    while l.exit.is_none() && t.elapsed() < Duration::from_secs(10) {
        l = b.cv.wait_timeout(l, Duration::from_millis(100)).unwrap().0;
    }
}

/// On by default; `JUSTRUST_BUILDD_PREBUILD=0` turns it off.
fn wants_prebuild(req: &Request) -> bool {
    !req.env.iter().any(|(k, v)| {
        k == "JUSTRUST_BUILDD_PREBUILD" && matches!(v.as_str(), "0" | "off" | "false")
    })
}

fn backoff_duration(misses: u32) -> Duration {
    match misses {
        0 => Duration::ZERO,
        1 => Duration::from_secs(15),
        2 => Duration::from_secs(60),
        3 => Duration::from_secs(300),
        _ => Duration::from_secs(900),
    }
}

fn record_prebuild_hit(st: &mut State, cwd: &str) {
    st.prebuild_backoff.remove(cwd);
}

fn record_prebuild_miss(st: &mut State, cwd: &str, now: Instant) {
    let entry = st.prebuild_backoff.entry(cwd.to_owned()).or_default();
    entry.misses = entry.misses.saturating_add(1);
    entry.cooldown_until = Some(now + backoff_duration(entry.misses));
}

fn in_prebuild_cooldown(st: &State, cwd: &str, now: Instant) -> bool {
    st.prebuild_backoff
        .get(cwd)
        .and_then(|b| b.cooldown_until)
        .is_some_and(|until| now < until)
}

/// Pick at most one eligible prebuild candidate per workspace (`cwd`),
/// preferring the most recently used command. Skips workspaces that already
/// have an in-flight build, are in adaptive backoff cooldown, or haven't
/// established a real edit-build loop yet (`seen < MIN_REQUESTS_TO_PREBUILD`,
/// unless explicitly queued by `warm`).
fn select_prebuild_candidates(
    st: &State,
    dirty: &[PathBuf],
    retry: &[String],
    queued: &[String],
    now: Instant,
) -> Vec<(String, Request, Vec<PathBuf>)> {
    let mut best_by_cwd: HashMap<&str, (&String, &Recent)> = HashMap::new();
    for (k, r) in &st.recent {
        if r.budget == 0 {
            continue;
        }
        let is_queued = queued.contains(k);
        if !is_queued && r.seen < MIN_REQUESTS_TO_PREBUILD {
            continue;
        }
        if !is_queued && in_prebuild_cooldown(st, &r.req.cwd, now) {
            continue;
        }
        if st.builds.values().any(|b| b.cwd == r.req.cwd) {
            continue;
        }
        let triggered = retry.contains(k)
            || is_queued
            || dirty
                .iter()
                .any(|d| r.roots.iter().any(|x| d.starts_with(x)));
        if !triggered {
            continue;
        }
        best_by_cwd
            .entry(r.req.cwd.as_str())
            .and_modify(|cur| {
                if r.used > cur.1.used {
                    *cur = (k, r);
                }
            })
            .or_insert((k, r));
    }
    best_by_cwd
        .into_values()
        .map(|(k, r)| (k.clone(), r.req.clone(), r.roots.clone()))
        .collect()
}

fn is_test(argv: &[String]) -> bool {
    matches!(argv.first().map(String::as_str), Some("test" | "t"))
}

/// Whether the prediction must change the invocation to avoid running tests.
/// An explicit Cargo --no-run already is the exact request: preserve its
/// argv, output and status so normal same-stamp attach/replay can serve it.
fn needs_compile_only(argv: &[String]) -> bool {
    is_test(argv)
        && !argv
            .iter()
            .take_while(|a| *a != "--")
            .any(|a| a == "--no-run")
}

/// `test ARGS -- FILTERS` -> `test ARGS --no-run`: the same units, no run.
fn no_run_argv(argv: &[String]) -> Vec<String> {
    let mut out: Vec<String> = argv.iter().take_while(|a| *a != "--").cloned().collect();
    if !out.iter().any(|a| a == "--no-run") {
        out.push("--no-run".into());
    }
    out
}

/// `test ARGS [FILTER] [-- LIBTEST ARGS]` -> `test ARGS`: what decides the
/// units Cargo compiles. `test --lib a::` and `test --lib -- b c` build the
/// same test binary and differ only in which of its tests run.
///
/// An unknown flag is taken to have no value. If it has one, two requests
/// that differ there count as the same units, and the cost is waiting for a
/// prebuild that helps less than hoped: the request still runs in full.
///
/// Flags that only change what Cargo prints or whether it keeps running
/// tests are dropped. The test pool compiles with
/// `test ARGS --no-run --message-format=json-render-diagnostics` before it
/// runs anything, so every pooled `justrust test` arrives in that form, while
/// the prebuild guessed from the previous request is `test ARGS --no-run`.
fn units_argv(argv: &[String]) -> Option<Vec<String>> {
    if !is_test(argv) {
        return None;
    }
    let mut out = vec!["test".to_owned()];
    let mut args = argv[1..].iter().take_while(|a| *a != "--");
    while let Some(a) = args.next() {
        if a == "--no-run" || !a.starts_with('-') {
            continue;
        }
        if matches!(a.as_str(), "--message-format" | "--color") {
            args.next();
            continue;
        }
        if a.starts_with("--message-format=")
            || a.starts_with("--color=")
            || matches!(
                a.as_str(),
                "--no-fail-fast" | "-q" | "--quiet" | "-v" | "-vv" | "--verbose"
            )
        {
            continue;
        }
        out.push(a.clone());
        if crate::findings::VALUE_FLAGS.contains(&a.as_str()) {
            out.extend(args.next().cloned());
        }
    }
    Some(out)
}

/// [`request_key`] of the units a `test` request compiles, whatever it runs.
fn units_key(req: &Request) -> Option<String> {
    let mut same = req.clone();
    same.argv = units_argv(&req.argv)?;
    Some(request_key(&same))
}

/// A compile-only prebuild for this agent is compiling, from these sources,
/// the units this request needs, guessed from a request for other tests of
/// them (`test --lib a::` before `test --lib b::`). Claim it, so that it is
/// not stopped as a wrong guess: the caller waits for it, and the real run
/// that follows finds every unit Fresh. Stopping it would discard its
/// progress and compile the same units again from the start.
fn claim_same_units(dm: &Daemon, req: &Request, key: &str, stamp: &str) -> Option<Arc<Build>> {
    let units = units_key(req)?;
    let session = session_of(req);
    let st = dm.state.lock().unwrap();
    let b = st
        .builds
        .iter()
        .find(|(k, b)| {
            *k != key
                && b.session == session
                && b.stamp.as_deref() == Some(stamp)
                && *b.compile_only.lock().unwrap()
                && b.units_key.lock().unwrap().as_deref() == Some(&*units)
                && b.log.lock().unwrap().exit.is_none()
        })
        .map(|(_, b)| b.clone())?;
    b.log.lock().unwrap().claimed = true;
    Some(b)
}

/// A compile-only prebuild of this request and these sources is running:
/// wait for it (the agent's build would wait on its slot anyway), so the
/// real run that follows finds every unit Fresh. A finished one is dropped.
/// Returns the prebuild of these sources whose units the run will reuse.
fn settle_compile_only(dm: &Daemon, key: &str, stamp: &str) -> Option<Arc<Build>> {
    let (done, b) = {
        let mut st = dm.state.lock().unwrap();
        let done = if st
            .finished
            .get(key)
            .is_some_and(|b| *b.compile_only.lock().unwrap())
        {
            st.finished
                .remove(key)
                .filter(|b| b.stamp.as_deref() == Some(stamp))
        } else {
            None
        };
        let b = st
            .builds
            .get(key)
            .filter(|b| b.stamp.as_deref() == Some(stamp) && *b.compile_only.lock().unwrap())
            .cloned();
        (done, b)
    };
    let Some(b) = b else { return done };
    b.log.lock().unwrap().claimed = true;
    log(&format!("waiting for compile-only prebuild pid {}", b.pid));
    let mut l = b.log.lock().unwrap();
    while l.exit.is_none() {
        l = b.cv.wait(l).unwrap();
    }
    drop(l);
    // `reap` forgets it; make sure it is gone before we look up `key`.
    let mut st = dm.state.lock().unwrap();
    if st.builds.get(key).is_some_and(|x| Arc::ptr_eq(x, &b)) {
        st.builds.remove(key);
    }
    drop(st);
    Some(b)
}

fn session_of(req: &Request) -> String {
    req.env
        .iter()
        .find(|(k, _)| k == "JUSTRUST_SLOT_SESSION")
        .map(|(_, v)| v.clone())
        .unwrap_or_default()
}

/// The agent asked for something other than what was prebuilt for it: the
/// guess was wrong, stop the prebuild so it does not compete.
fn cancel_wrong_guess(dm: &Daemon, key: &str, session: &str) {
    let wrong: Vec<Arc<Build>> = {
        let st = dm.state.lock().unwrap();
        st.builds
            .iter()
            .filter(|(k, b)| wrong_guess_candidate(k, b, key, session))
            .map(|(_, b)| b.clone())
            .collect()
    };
    for b in wrong {
        log(&format!(
            "agent asked for another build, stopping prebuild pid {}",
            b.pid
        ));
        stop(&b);
    }
}

/// A build of this request that the daemon stopped has not exited yet. It
/// still holds the slot and the target directory, and its exit is not a
/// result: wait for it to go, then the request starts its own build.
fn await_stopped(dm: &Daemon, key: &str) {
    let dying = {
        let st = dm.state.lock().unwrap();
        st.builds.get(key).filter(|b| stopped(b)).cloned()
    };
    let Some(b) = dying else { return };
    log(&format!("waiting for stopped build pid {} to exit", b.pid));
    let mut l = b.log.lock().unwrap();
    let t = Instant::now();
    while l.exit.is_none() && t.elapsed() < STOP_WAIT {
        l = b.cv.wait_timeout(l, Duration::from_millis(100)).unwrap().0;
    }
    drop(l);
    // `reap` forgets it; make sure it is gone before `key` is looked up.
    let mut st = dm.state.lock().unwrap();
    if st.builds.get(key).is_some_and(|x| Arc::ptr_eq(x, &b)) {
        st.builds.remove(key);
    }
}

/// How long a request waits for a build the daemon stopped to exit.
const STOP_WAIT: Duration = Duration::from_secs(10);

fn prune_recent(recent: &mut HashMap<String, Recent>) {
    recent.retain(|_, r| r.used.elapsed() < RECENT_MAX_AGE);
    // One per agent session: its latest request.
    let mut latest: HashMap<String, Instant> = HashMap::new();
    for r in recent.values() {
        let e = latest.entry(session_of(&r.req)).or_insert(r.used);
        *e = (*e).max(r.used);
    }
    recent.retain(|_, r| latest.get(&session_of(&r.req)) == Some(&r.used));
    while recent.len() > RECENT_MAX {
        let Some(oldest) = recent
            .iter()
            .min_by_key(|(_, r)| r.used)
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        recent.remove(&oldest);
    }
}

/// Watch every source directory under `roots` (build output and hidden
/// directories excluded, like the source stamp).
fn watch_roots(dm: &Daemon, roots: &[PathBuf]) {
    let Some(w) = &dm.watcher else { return };
    let mut watched = dm.watched.lock().unwrap();
    for r in roots {
        if !watched.insert(r.clone()) {
            continue;
        }
        let mut dirs = Vec::new();
        let mut stack = vec![r.clone()];
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let n = e.file_name();
                let n = n.to_string_lossy();
                if e.file_type().is_ok_and(|t| t.is_dir())
                    && n != "target"
                    && !n.starts_with("target-")
                    && !n.starts_with('.')
                {
                    stack.push(e.path());
                }
            }
            dirs.push(d);
        }
        let n = w.watch_all(&dirs);
        log(&format!("watching {} ({n} directories)", r.display()));
    }
}

/// The prebuild seam: when sources under a remembered request's roots
/// change, start that request as a prebuild once the burst of events is
/// over. A request whose previous build is still running is retried after
/// it finishes (its sources are already stale).
fn prebuild_loop(dm: &Arc<Daemon>, rx: std::sync::mpsc::Receiver<PathBuf>) {
    let mut dirty: Vec<PathBuf> = Vec::new();
    let mut retry: Vec<String> = Vec::new();
    loop {
        let timeout = if dirty.is_empty() && retry.is_empty() {
            Duration::from_secs(3600)
        } else {
            DEBOUNCE
        };
        match rx.recv_timeout(timeout) {
            Ok(p) => {
                dirty.push(p);
                continue;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
        }
        // Queued by `warm`: prebuilt whatever the watcher saw.
        let queued = dm.warm.take_ready();
        let wanted: Vec<(String, Request, Vec<PathBuf>)> = {
            let mut st = dm.state.lock().unwrap();
            let now = Instant::now();
            prune_recent(&mut st.recent);
            let mut expired_cwds = Vec::new();
            st.finished.retain(|_, b| {
                let keep = b.started.elapsed() < FINISHED_MAX_AGE;
                if !keep && *b.prebuild.lock().unwrap() && !b.log.lock().unwrap().claimed {
                    expired_cwds.push(b.cwd.clone());
                }
                keep
            });
            for cwd in expired_cwds {
                record_prebuild_miss(&mut st, &cwd, now);
            }
            select_prebuild_candidates(&st, &dirty, &retry, &queued, now)
        };
        dirty.clear();
        retry.clear();
        if dm.draining.load(std::sync::atomic::Ordering::SeqCst) || dm.exe_replaced() {
            drain_and_exit(dm, "binary replaced on disk, stopping prebuilds");
            return;
        }
        for (key, mut req, roots) in wanted {
            let cwd = PathBuf::from(&req.cwd);
            if forget_if_gone(&mut dm.state.lock().unwrap().recent, &key, &cwd) {
                continue;
            }
            let (stamp, _) = source_stamp(&roots, &cwd, &req.env);
            // An unclaimed prebuild of older sources is useless now.
            cancel_stale_prebuild(dm, &key, &stamp);
            let mut st = dm.state.lock().unwrap();
            if let Some(b) = st.builds.get(&key) {
                if b.stamp.as_deref() != Some(&*stamp) {
                    retry.push(key);
                }
                continue;
            }
            if st
                .finished
                .get(&key)
                .is_some_and(|b| b.stamp.as_deref() == Some(&*stamp))
            {
                continue;
            }
            req.prebuild = true;
            // Never run an agent's tests on a guess: they may touch files,
            // sockets or processes, and a cancelled prebuild would kill
            // them mid-run. Compile the same units only.
            let compile_only = needs_compile_only(&req.argv);
            let units = compile_only.then(|| units_key(&req)).flatten();
            if compile_only {
                req.argv = no_run_argv(&req.argv);
            }
            // Its own run id: never the pinned id of the request it copies.
            // A pinned id also keeps the router from sending it remote.
            req.env.retain(|(k, _)| k != "JUSTRUST_RUN_ID_OVERRIDE");
            let id = format!("{}-pre", crate::record::new_run_id());
            req.env.push(("JUSTRUST_RUN_ID_OVERRIDE".into(), id));
            // Nobody waits for it: not an agent build in history and trend
            // statistics. The slot stays the agent's (JUSTRUST_SLOT_SESSION).
            req.env.retain(|(k, _)| k != "JCODE_SESSION_ID");
            req.env.retain(|(k, _)| k != GUESS_ENV);
            req.env.push((GUESS_ENV.into(), "1".into()));
            if queued.contains(&key) {
                warm::mark(&mut req);
            }
            // One prebuild of a command in a build directory at a time, over
            // all daemons: wait for the other one, then find its units Fresh.
            let claim = match dir().map(|d| prebuild_claim(&d.join("prebuild"), &req)) {
                Ok(Err(Busy)) => {
                    retry.push(key);
                    continue;
                }
                Ok(Ok(f)) => f,
                Err(_) => None,
            };
            match start(&req, Some(stamp), &cwd) {
                Ok(b) => {
                    *b.claim.lock().unwrap() = claim;
                    *b.compile_only.lock().unwrap() = compile_only;
                    *b.units_key.lock().unwrap() = units;
                    // The pid ties this line to its exit line, the key to
                    // the request it was guessed from.
                    log(&format!(
                        "prebuild {} in {} (pid {}, key {}, stamp {})",
                        req.argv.join(" "),
                        req.cwd,
                        b.pid,
                        key.get(..8).unwrap_or(&key),
                        b.stamp.as_deref().map(|s| &s[..8]).unwrap_or("native")
                    ));
                    if let Some(r) = st.recent.get_mut(&key) {
                        r.budget = r.budget.saturating_sub(1);
                    }
                    st.builds.insert(key.clone(), b.clone());
                    let (dm2, b2) = (dm.clone(), b.clone());
                    std::thread::spawn(move || reap(&dm2, &key, &b2));
                }
                Err(e) => log(&format!(
                    "prebuild failed to start in {}: {e:#}",
                    cwd.display()
                )),
            }
        }
    }
}

/// Forget the build once it finished, so later requests start a new one.
fn reap(dm: &Daemon, key: &str, b: &Arc<Build>) {
    let code = {
        let mut l = b.log.lock().unwrap();
        while l.exit.is_none() {
            l = b.cv.wait(l).unwrap();
        }
        l.exit
    };
    // The next prebuild of this command here may start.
    b.claim.lock().unwrap().take();
    let mut st = dm.state.lock().unwrap();
    if st.builds.get(key).is_some_and(|x| Arc::ptr_eq(x, b)) {
        st.builds.remove(key);
    }
    // Nobody saw it: keep it for the request it was started for.
    let l = b.log.lock().unwrap();
    let (claimed, early_served, sharers) = (l.claimed, l.early_served, l.sharers);
    let dropped = no_result(b, &l);
    drop(l);
    let was_prebuild = *b.prebuild.lock().unwrap();
    if was_prebuild && (claimed || sharers > 0) {
        record_prebuild_hit(&mut st, &b.cwd);
    } else if was_prebuild && !claimed && dropped {
        record_prebuild_miss(&mut st, &b.cwd, Instant::now());
    }
    if b.stamp.is_some() && !dropped && ((!claimed && was_prebuild) || early_served) {
        st.finished.insert(key.to_owned(), b.clone());
    }
    st.last_request = Some(Instant::now());
    drop(st);
    log(&format!(
        "pid {} exit {:?} after {:.2}s, shared by {}",
        b.pid,
        code,
        b.started.elapsed().as_secs_f64(),
        b.log.lock().unwrap().sharers
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// The stamp before mirrors were recognised, for the paired profile.
        pub(super) static STAMP_EVERYTHING: std::cell::Cell<bool> =
            const { std::cell::Cell::new(false) };
        /// `justrust local off` on the machine under test.
        pub(super) static LOCAL_OFF: std::cell::Cell<bool> =
            const { std::cell::Cell::new(false) };
    }

    #[test]
    fn each_build_has_its_own_daemon_socket() {
        let a = build_stem(&format!("sha256-{}", "ab12".repeat(16)));
        let b = build_stem(&format!("sha256-{}", "cd34".repeat(16)));
        assert_eq!(a, "ab12ab12ab12ab12");
        assert_ne!(a, b);
        // An unreadable binary has a fresh id per process: one shared name,
        // and the daemon behind it still refuses clients of another build.
        assert_eq!(build_stem("unreadable-12-34-0"), "unidentified");
        assert_eq!(build_stem("sha256-../../etc"), "unidentified");
        // sun_path holds 108 bytes: a home this deep still fits.
        let deep = Path::new("/home")
            .join("u".repeat(48))
            .join(".justrust/buildd");
        assert!(deep.join(format!("{a}.sock")).as_os_str().len() < 108);
    }

    #[test]
    fn sweep_removes_only_unheld_files_of_other_builds() {
        let d = std::env::temp_dir().join(format!("jr-buildd-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        for f in [
            "mine.lock",
            "mine.sock",
            "gone.lock",
            "gone.sock",
            "live.lock",
            "live.sock",
            "lock",
            "sock",
            "daemon.log",
        ] {
            std::fs::write(d.join(f), b"").unwrap();
        }
        // A daemon of another build is running: it holds its lock.
        let live = std::fs::File::open(d.join("live.lock")).unwrap();
        live.try_lock().unwrap();
        // Nothing is old enough yet.
        sweep_stale(&d, &d.join("mine.lock"), Duration::from_secs(3600));
        assert!(d.join("gone.lock").exists() && d.join("gone.sock").exists());
        sweep_stale(&d, &d.join("mine.lock"), Duration::ZERO);
        let mut left: Vec<String> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        // The legacy shared pair belongs to older clients: never touched.
        assert_eq!(
            left,
            [
                "daemon.log",
                "live.lock",
                "live.sock",
                "lock",
                "mine.lock",
                "mine.sock",
                "sock"
            ]
        );
        drop(live);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    #[cfg(unix)]
    fn stamp_component_keys_match_path_order_and_restore_bytes() {
        use std::os::unix::ffi::OsStringExt;
        for root in ["", ".", "./", "/", "//", "a/./b/../c//", "/a//./b/../c/"] {
            let mut entries = Vec::new();
            for suffix in ["a", "a/b", "a.b", "a-/b", ".hidden/x", "aa", "a../x"] {
                entries.push((Path::new(root).join(suffix), entries.len()));
            }
            let mut state = 0x1234_5678_9abc_def0u64;
            for index in 0..4000 {
                let mut path = PathBuf::from(root);
                for _ in 0..(index % 7 + 1) {
                    let mut name = Vec::new();
                    for _ in 0..(index % 13 + 1) {
                        state ^= state << 13;
                        state ^= state >> 7;
                        state ^= state << 17;
                        let mut byte = (state % 255 + 1) as u8;
                        if byte == b'/' {
                            byte = b'_';
                        }
                        name.push(byte);
                    }
                    if name == b"." || name == b".." {
                        name.push(b'_');
                    }
                    path.push(std::ffi::OsString::from_vec(name));
                }
                entries.push((path, entries.len()));
            }
            let mut oracle = entries.clone();
            oracle.sort_by(|a, b| a.0.cmp(&b.0));
            sort_stamp_root_entries(&mut entries);
            assert_eq!(entries, oracle, "root {root:?}");
        }
    }

    #[test]
    #[ignore = "controlled same-metadata full-checkout sort and hash oracle"]
    fn source_stamp_sort_oracle_checkout() {
        let cwd = std::env::var_os("JUSTRUST_STAMP_PROFILE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let roots = source_roots(&cwd);
        let env: Vec<_> = std::env::vars().collect();
        let hash = |entries: &[(PathBuf, std::fs::Metadata)]| {
            let mut h = crate::depcache::H128::new();
            for (path, metadata) in entries {
                h.field(path.as_os_str().as_encoded_bytes());
                h.field(&metadata.len().to_le_bytes());
                let (s, ns) = crate::sys::mtime(metadata);
                h.field(&s.to_le_bytes());
                h.field(&ns.to_le_bytes());
                h.field(&crate::sys::file_id(metadata).1.to_le_bytes());
            }
            h.hex()
        };
        for sample in 0..5 {
            source_stamp_with_sort(
                &roots,
                &cwd,
                &env,
                |_| {},
                |entries| {
                    let mut oracle = entries.to_vec();
                    let start = Instant::now();
                    oracle.sort_by(|a, b| a.0.cmp(&b.0));
                    let old_ms = start.elapsed().as_secs_f64() * 1000.0;
                    let start = Instant::now();
                    sort_stamp_root_entries(entries);
                    let new_ms = start.elapsed().as_secs_f64() * 1000.0;
                    assert!(
                        entries
                            .iter()
                            .zip(&oracle)
                            .all(|(a, b)| a.0.as_os_str() == b.0.as_os_str())
                    );
                    let expected = hash(&oracle);
                    assert_eq!(hash(entries), expected);
                    eprintln!(
                        "stamp-sort-oracle sample={sample} files={} old_ms={old_ms:.3} new_ms={new_ms:.3} identical_hash={expected}",
                        entries.len()
                    );
                },
            );
        }
    }

    #[test]
    #[ignore = "controlled full-checkout source stamp timing diagnostic"]
    fn source_stamp_profile_checkout() {
        let cwd = std::env::var_os("JUSTRUST_STAMP_PROFILE_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let roots = source_roots(&cwd);
        let env: Vec<_> = std::env::vars().collect();
        // Interleaved: what a mirror's `.justrustignore` saves on this tree.
        for sample in 0..10 {
            let everything = sample % 2 == 0;
            STAMP_EVERYTHING.with(|on| on.set(everything));
            let start = Instant::now();
            let mut last = start;
            let mut timings = std::collections::BTreeMap::<&str, Duration>::new();
            let (stamp, files) = source_stamp_observed(&roots, &cwd, &env, |name| {
                let now = Instant::now();
                if name != "root-start" {
                    *timings.entry(name).or_default() += now.duration_since(last);
                }
                last = now;
            });
            eprintln!(
                "stamp-profile sample={sample} mirror_rules={} files={files} stamp={stamp} total_ms={:.3} phases_ms={:?}",
                !everything,
                start.elapsed().as_secs_f64() * 1000.0,
                timings
                    .iter()
                    .map(|(name, time)| (*name, time.as_secs_f64() * 1000.0))
                    .collect::<Vec<_>>()
            );
        }
        STAMP_EVERYTHING.with(|on| on.set(false));
    }

    #[test]
    #[ignore = "paired benchmark on a quiescent checkout"]
    fn stamp_stream_pair() {
        let cwd = std::env::current_dir().unwrap();
        let roots = source_roots(&cwd);
        let env: Vec<_> = std::env::vars().collect();
        let scan = |stream| {
            let t = Instant::now();
            let result = if stream {
                source_stamp(&roots, &cwd, &env)
            } else {
                source_stamp_with_sort(&roots, &cwd, &env, |_| {}, sort_stamp_root_entries)
            };
            (result, t.elapsed().as_millis())
        };
        for i in 0..6 {
            let (a, b) = (scan(i % 2 == 0), scan(i % 2 != 0));
            assert_eq!(a.0, b.0);
            let (old, new) = if i % 2 == 0 { (b.1, a.1) } else { (a.1, b.1) };
            eprintln!(
                "stamp-stream-pair files={} old_ms={old} new_ms={new}",
                a.0.1
            );
        }
    }

    fn req(env: &[(&str, &str)], argv: &[&str]) -> Request {
        Request {
            build: String::new(),
            cwd: "/w".into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env: env
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            prebuild: false,
        }
    }

    #[test]
    fn agent_identity_does_not_split_requests() {
        let a = req(
            &[
                ("JCODE_SESSION_ID", "a"),
                ("PATH", "/bin"),
                ("KITTY_PID", "1"),
            ],
            &["test", "--lib"],
        );
        let b = req(
            &[
                ("JCODE_SESSION_ID", "b"),
                ("PATH", "/bin"),
                ("KITTY_PID", "2"),
            ],
            &["test", "--lib"],
        );
        assert_eq!(request_key(&a), request_key(&b));
    }

    #[test]
    fn build_env_and_args_split_requests() {
        let base = req(&[("PATH", "/bin")], &["test", "--lib"]);
        let flags = req(
            &[("PATH", "/bin"), ("RUSTFLAGS", "-Copt-level=1")],
            &["test", "--lib"],
        );
        let filter = req(&[("PATH", "/bin")], &["test", "--lib", "--", "x"]);
        let frontend = req(
            &[("PATH", "/bin"), ("JCODE_FRONTEND_THREADS", "4")],
            &["test", "--lib"],
        );
        let k = request_key(&base);
        assert_ne!(k, request_key(&flags));
        assert_ne!(k, request_key(&filter));
        assert_ne!(k, request_key(&frontend));
    }

    #[test]
    fn one_prebuild_of_a_command_in_a_directory_at_a_time_across_daemons() {
        // Hosted runs 20261121-232520069-1051429-pre and
        // 20261122-072519783-793907-pre: two daemons, same check, same
        // directory, same instant.
        let dir = std::env::temp_dir().join(format!("jr-claim-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let req = |argv: &[&str], session: &str, port: &str| Request {
            build: String::new(),
            prebuild: true,
            cwd: "/w".into(),
            argv: argv.iter().map(|s| s.to_string()).collect(),
            env: vec![
                ("JUSTRUST_SLOT_SESSION".into(), session.into()),
                ("UNRELATED".into(), port.into()),
            ],
        };
        let held = prebuild_claim(&dir, &req(&["check"], "s", "1")).unwrap();
        assert!(held.is_some());
        // Another daemon (its own open file), or another key in this one.
        assert_eq!(
            prebuild_claim(&dir, &req(&["check"], "s", "2")).err(),
            Some(Busy)
        );
        // Another command, or another session's slot, is not held back.
        assert!(prebuild_claim(&dir, &req(&["test"], "s", "1")).is_ok());
        assert!(prebuild_claim(&dir, &req(&["check"], "t", "1")).is_ok());
        // The holder exited (or died: the lock goes with the file).
        drop(held);
        assert!(prebuild_claim(&dir, &req(&["check"], "s", "2")).is_ok());
        // Nowhere to keep claims: prebuild anyway.
        let file = dir.join("not-a-directory");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(
            prebuild_claim(&file, &req(&["check"], "s", "1")).map(|f| f.is_some()),
            Ok(false)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_deleted_workspace_is_forgotten_instead_of_prebuilt() {
        // Hosted daemon log 2026-11-27: 40 "prebuild failed to start: No
        // such file or directory", after scratch workspaces were removed.
        let dir = std::env::temp_dir().join(format!("jr-gone-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut recent = HashMap::from([("k".to_string(), 1), ("other".to_string(), 2)]);
        assert!(!forget_if_gone(&mut recent, "k", &dir));
        assert_eq!(recent.len(), 2);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(forget_if_gone(&mut recent, "k", &dir));
        assert_eq!(recent.keys().collect::<Vec<_>>(), ["other"]);
        // Asked again for the same key: still skipped, nothing left to drop.
        assert!(forget_if_gone(&mut recent, "k", &dir));
    }

    #[test]
    fn the_connection_a_hosted_client_came_in_on_does_not_split_requests() {
        // Two release jobs, or one client before and after its link dropped:
        // the same build, so a prebuild of it is attached to, not raced for
        // the build directory (20261127-002034387-2928 waited 25.8s there).
        let argv = [
            "build",
            "--release",
            "--target",
            "aarch64-unknown-linux-musl",
        ];
        let first = req(
            &[
                ("PATH", "/bin"),
                ("SSH_CLIENT", "50.189.71.253 34608 22"),
                ("SSH_CONNECTION", "50.189.71.253 34608 100.64.1.2 22"),
            ],
            &argv,
        );
        let second = req(
            &[
                ("PATH", "/bin"),
                ("SSH_CLIENT", "20.1.2.3 51022 22"),
                ("SSH_CONNECTION", "20.1.2.3 51022 100.64.1.2 22"),
                ("SSH_TTY", "/dev/pts/3"),
            ],
            &argv,
        );
        assert_eq!(request_key(&first), request_key(&second));
        assert_eq!(
            request_key(&first),
            request_key(&req(&[("PATH", "/bin")], &argv))
        );
    }

    #[test]
    fn test_prebuilds_never_run_tests() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            no_run_argv(&v(&[
                "test",
                "-p",
                "x",
                "--lib",
                "--",
                "fps",
                "--nocapture"
            ])),
            v(&["test", "-p", "x", "--lib", "--no-run"])
        );
        assert_eq!(
            no_run_argv(&v(&["test", "--no-run"])),
            v(&["test", "--no-run"])
        );
        assert!(needs_compile_only(&v(&["t", "--", "--no-run"])));
        assert!(!needs_compile_only(&v(&[
            "test",
            "--no-run",
            "--message-format=json-render-diagnostics"
        ])));
        assert!(!needs_compile_only(&v(&["check"])));
    }

    #[test]
    fn other_tests_of_the_same_units_share_a_units_key() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let units = |a: &[&str]| units_argv(&v(a));
        let lib = Some(v(&["test", "-p", "x", "--lib"]));
        assert_eq!(units(&["test", "-p", "x", "--lib"]), lib);
        assert_eq!(units(&["test", "-p", "x", "--lib", "fps::"]), lib);
        assert_eq!(units(&["test", "fps", "-p", "x", "--lib", "--no-run"]), lib);
        assert_eq!(
            units(&["test", "-p", "x", "--lib", "--", "a", "b", "--nocapture"]),
            lib
        );
        // A flag's value is not a filter, and what is compiled differs.
        assert_ne!(units(&["test", "-p", "y", "--lib"]), lib);
        assert_ne!(units(&["test", "-p", "x"]), lib);
        assert_ne!(units(&["test", "-p", "x", "--lib", "--release"]), lib);
        assert_ne!(units(&["test", "--package=x", "--lib"]), lib);
        assert_eq!(units(&["check", "-p", "x", "--lib"]), None);
        // The pool's compile request, and other flags that only change what
        // is printed or whether tests keep running.
        assert_eq!(
            units(&[
                "test",
                "-p",
                "x",
                "--lib",
                "--no-run",
                "--message-format=json-render-diagnostics"
            ]),
            lib
        );
        assert_eq!(
            units(&[
                "test",
                "--message-format",
                "json",
                "--color",
                "never",
                "-p",
                "x",
                "--lib",
                "-q",
                "--no-fail-fast",
                "--color=always"
            ]),
            lib
        );

        let key = |env: &[(&str, &str)], a: &[&str]| units_key(&req(env, a));
        let base = key(&[("PATH", "/bin")], &["test", "--lib", "a::"]);
        assert!(base.is_some());
        assert_eq!(
            base,
            key(&[("PATH", "/bin")], &["test", "--lib", "--", "b"])
        );
        assert_ne!(
            base,
            key(&[("PATH", "/usr/bin")], &["test", "--lib", "a::"])
        );
        assert_ne!(base, key(&[("PATH", "/bin")], &["test", "a::"]));
    }

    #[test]
    fn a_prebuild_of_the_same_units_is_claimed_not_stopped() {
        let dm = Daemon {
            state: Mutex::default(),
            watcher: None,
            watched: Mutex::default(),
            sock: (PathBuf::new(), (0, 0)),
            exe: None,
            draining: std::sync::atomic::AtomicBool::new(false),
            warm: Default::default(),
        };
        let env = [("PATH", "/bin"), ("JUSTRUST_SLOT_SESSION", "agent")];
        let guessed = req(&env, &["test", "--lib", "--", "a::"]);
        let asked = req(&env, &["test", "--lib", "--", "b::", "c"]);
        let (guessed_key, asked_key) = (request_key(&guessed), request_key(&asked));
        assert_ne!(guessed_key, asked_key);
        let prebuild = |units: Option<String>| {
            let mut b = fake_build();
            b.session = "agent".into();
            *b.prebuild.lock().unwrap() = true;
            *b.compile_only.lock().unwrap() = true;
            *b.units_key.lock().unwrap() = units;
            Arc::new(b)
        };
        let insert = |b: &Arc<Build>| {
            let mut st = dm.state.lock().unwrap();
            st.builds.clear();
            st.builds.insert(guessed_key.clone(), b.clone());
        };

        // Other tests of the same units, same sources: wait for it.
        let b = prebuild(units_key(&guessed));
        insert(&b);
        assert!(wrong_guess_candidate(&guessed_key, &b, &asked_key, "agent"));
        let claimed = claim_same_units(&dm, &asked, &asked_key, "s").unwrap();
        assert!(Arc::ptr_eq(&claimed, &b));
        assert!(!wrong_guess_candidate(
            &guessed_key,
            &b,
            &asked_key,
            "agent"
        ));

        // Older sources, other units, another agent, or already over: a
        // wrong guess as before.
        let unclaimed = |b: &Arc<Build>, asked: &Request, stamp: &str| {
            insert(b);
            let key = request_key(asked);
            assert!(claim_same_units(&dm, asked, &key, stamp).is_none());
            assert!(!b.log.lock().unwrap().claimed);
        };
        unclaimed(&prebuild(units_key(&guessed)), &asked, "newer");
        unclaimed(
            &prebuild(units_key(&guessed)),
            &req(&env, &["test", "--", "b::"]),
            "s",
        );
        unclaimed(
            &prebuild(units_key(&guessed)),
            &req(&env, &["clippy", "--all-targets"]),
            "s",
        );
        unclaimed(&prebuild(None), &asked, "s");
        let other = prebuild(units_key(&guessed));
        other.log.lock().unwrap().exit = Some(0);
        unclaimed(&other, &asked, "s");
        let mut b = fake_build();
        b.session = "other".into();
        *b.prebuild.lock().unwrap() = true;
        *b.compile_only.lock().unwrap() = true;
        *b.units_key.lock().unwrap() = units_key(&guessed);
        unclaimed(&Arc::new(b), &asked, "s");
    }

    #[test]
    fn native_empty_requires_opt_in_and_no_candidates() {
        let mut st = State::default();
        let mut r = req(&[], &["check"]);
        assert!(!native_empty_eligible(&st, &r));
        r.env
            .push(("JUSTRUST_BUILDD_NATIVE_EMPTY".into(), "0".into()));
        assert!(!native_empty_eligible(&st, &r));
        r.env[0].1 = "1".into();
        assert!(native_empty_eligible(&st, &r));
        r.prebuild = true;
        assert!(!native_empty_eligible(&st, &r));
        r.prebuild = false;
        let b = Arc::new(fake_build());
        st.builds.insert("other-key".into(), b.clone());
        assert!(!native_empty_eligible(&st, &r));
        st.builds.clear();
        st.finished.insert("other-key".into(), b);
        assert!(!native_empty_eligible(&st, &r));
    }

    #[test]
    fn native_no_candidate_defaults_to_foreground_metadata_checks() {
        let st = State::default();
        for cmd in ["check", "c", "clippy", "build", "test", "t"] {
            assert_eq!(
                native_no_candidate_eligible(&st, &req(&[], &[cmd]), "key"),
                matches!(cmd, "check" | "c" | "clippy")
            );
        }
        let mut r = req(&[], &["check"]);
        for value in ["0", "true", "on", "01", "", "1"] {
            r.env = vec![("JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE".into(), value.into())];
            assert_eq!(native_no_candidate_eligible(&st, &r, "key"), value == "1");
        }
        r.prebuild = true;
        assert!(!native_no_candidate_eligible(&st, &r, "key"));
    }

    #[test]
    fn native_no_candidate_blocks_every_same_key_owner_and_result() {
        let r = req(&[("JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE", "1")], &["check"]);
        for stamped in [false, true] {
            for complete in [false, true] {
                for compile_only in [false, true] {
                    let mut st = State::default();
                    let mut b = fake_build();
                    if !stamped {
                        b.stamp = None;
                    }
                    *b.compile_only.lock().unwrap() = compile_only;
                    b.log.lock().unwrap().exit = complete.then_some(0);
                    st.builds.insert("key".into(), Arc::new(b));
                    assert!(!native_no_candidate_eligible(&st, &r, "key"));
                    st.finished = std::mem::take(&mut st.builds);
                    assert!(!native_no_candidate_eligible(&st, &r, "key"));
                }
            }
        }
    }

    #[test]
    fn native_no_candidate_matches_wrong_guess_operations_not_roots() {
        for session in ["", "agent"] {
            let r = req(
                &[
                    ("JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE", "1"),
                    ("JUSTRUST_SLOT_SESSION", session),
                ],
                &["check"],
            );
            for same_session in [false, true] {
                for prebuild in [false, true] {
                    for claimed in [false, true] {
                        for early_served in [false, true] {
                            for complete in [false, true] {
                                let mut st = State::default();
                                let mut b = fake_build();
                                b.session = if same_session { session } else { "other" }.into();
                                *b.prebuild.lock().unwrap() = prebuild;
                                {
                                    let mut l = b.log.lock().unwrap();
                                    l.claimed = claimed;
                                    l.early_served = early_served;
                                    l.exit = complete.then_some(0);
                                }
                                let needs_cancel = same_session
                                    && !complete
                                    && ((prebuild && !claimed) || early_served);
                                assert_eq!(
                                    wrong_guess_candidate("other-key", &b, "key", session),
                                    needs_cancel
                                );
                                st.builds.insert("other-key".into(), Arc::new(b));
                                assert_eq!(
                                    native_no_candidate_eligible(&st, &r, "key"),
                                    !needs_cancel
                                );
                                // Finished other-key entries are never touched by
                                // the skipped request lookup or cancellation.
                                st.finished = std::mem::take(&mut st.builds);
                                assert!(native_no_candidate_eligible(&st, &r, "key"));
                                assert!(!native_empty_eligible(&st, &r));
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn orphaned_build_keeps_running_only_after_an_early_answer() {
        let l = |clients, exit, early_served| Log {
            clients,
            exit,
            early_served,
            ..Log::default()
        };
        assert!(stop_when_orphaned(&l(0, None, false), false));
        // After an early answer the build must finish for the replay.
        assert!(!stop_when_orphaned(&l(0, None, true), false));
        assert!(!stop_when_orphaned(&l(0, None, false), true));
        assert!(!stop_when_orphaned(&l(1, None, false), false));
        assert!(!stop_when_orphaned(&l(0, Some(0), false), false));
    }

    #[test]
    fn unstamped_execution_is_reserved_but_never_replayed() {
        let dm = Daemon {
            state: Mutex::default(),
            watcher: None,
            watched: Mutex::default(),
            sock: (PathBuf::new(), (0, 0)),
            exe: None,
            draining: std::sync::atomic::AtomicBool::new(false),
            warm: Default::default(),
        };
        let mut b = fake_build();
        b.stamp = None;
        // Even early output and prebuild flags cannot turn an unvalidated
        // execution into a replayable result.
        *b.prebuild.lock().unwrap() = true;
        b.log.lock().unwrap().early_served = true;
        b.log.lock().unwrap().exit = Some(101);
        let b = Arc::new(b);
        dm.state
            .lock()
            .unwrap()
            .builds
            .insert("key".into(), b.clone());
        assert!(native_key_occupied(&dm.state.lock().unwrap(), "key"));
        reap(&dm, "key", &b);
        let st = dm.state.lock().unwrap();
        assert!(st.builds.is_empty());
        assert!(st.finished.is_empty());
    }

    #[test]
    fn native_owner_refuses_racing_requests_without_overwrite() {
        let dm = Arc::new(Daemon {
            state: Mutex::default(),
            watcher: None,
            watched: Mutex::default(),
            sock: (PathBuf::new(), (0, 0)),
            exe: None,
            draining: std::sync::atomic::AtomicBool::new(false),
            warm: Default::default(),
        });
        let mut owner = fake_build();
        owner.stamp = None;
        let owner = Arc::new(owner);
        for (prebuild, flag) in [false, true].into_iter().flat_map(|prebuild| {
            [
                None,
                Some("JUSTRUST_BUILDD_NATIVE_EMPTY"),
                Some("JUSTRUST_BUILDD_NATIVE_NO_CANDIDATE"),
            ]
            .into_iter()
            .map(move |flag| (prebuild, flag))
        }) {
            let env: Vec<_> = flag.into_iter().map(|flag| (flag, "1")).collect();
            let mut r = req(&env, &["check"]);
            r.prebuild = prebuild;
            let key = request_key(&r);
            dm.state
                .lock()
                .unwrap()
                .builds
                .insert(key.clone(), owner.clone());
            let (server, mut client) = UnixStream::pair().unwrap();
            run(&dm, r, server).unwrap();
            let (kind, body) = read_msg(&mut client).unwrap().unwrap();
            assert_eq!(kind, b'F');
            assert_eq!(body, b"native freshness build already owns this request");
            let st = dm.state.lock().unwrap();
            assert!(Arc::ptr_eq(st.builds.get(&key).unwrap(), &owner));
            assert!(st.finished.is_empty());
            assert_eq!(st.served, 0);
            assert_eq!(st.coalesced, 0);
        }
    }

    #[test]
    fn native_reaper_preserves_another_active_reservation() {
        let dm = Daemon {
            state: Mutex::default(),
            watcher: None,
            watched: Mutex::default(),
            sock: (PathBuf::new(), (0, 0)),
            exe: None,
            draining: std::sync::atomic::AtomicBool::new(false),
            warm: Default::default(),
        };
        let mut old = fake_build();
        old.stamp = None;
        old.log.lock().unwrap().exit = Some(0);
        let active = Arc::new(fake_build());
        dm.state
            .lock()
            .unwrap()
            .builds
            .insert("key".into(), active.clone());
        reap(&dm, "key", &Arc::new(old));
        let st = dm.state.lock().unwrap();
        assert!(Arc::ptr_eq(st.builds.get("key").unwrap(), &active));
        assert!(st.finished.is_empty());
    }

    #[test]
    fn a_guess_that_declined_to_build_is_never_kept_as_a_result() {
        let dm = Daemon {
            state: Mutex::default(),
            watcher: None,
            watched: Mutex::default(),
            sock: (PathBuf::new(), (0, 0)),
            exe: None,
            draining: std::sync::atomic::AtomicBool::new(false),
            warm: Default::default(),
        };
        for (code, kept) in [(GUESS_DROPPED, false), (0, true), (101, true)] {
            let b = fake_build();
            *b.prebuild.lock().unwrap() = true;
            b.log.lock().unwrap().exit = Some(code);
            reap(&dm, "key", &Arc::new(b));
            let mut st = dm.state.lock().unwrap();
            assert_eq!(st.finished.contains_key("key"), kept, "exit {code}");
            st.finished.clear();
        }
    }

    fn bare_daemon() -> Daemon {
        Daemon {
            state: Mutex::default(),
            watcher: None,
            watched: Mutex::default(),
            sock: (PathBuf::new(), (0, 0)),
            exe: None,
            draining: std::sync::atomic::AtomicBool::new(false),
            warm: Default::default(),
        }
    }

    /// The clippy prebuild that a `test` request stopped exited 143; the
    /// clippy request that followed must build, not get 143 as its result.
    #[test]
    fn a_build_the_daemon_stopped_is_never_kept_or_answered() {
        let dm = bare_daemon();
        for (prebuild, claimed, code) in [(true, false, 143), (true, true, 0), (false, true, 143)] {
            let b = fake_build();
            *b.prebuild.lock().unwrap() = prebuild;
            {
                let mut l = b.log.lock().unwrap();
                l.claimed = claimed;
                // It answered early, which alone would keep its verdict.
                l.early_served = true;
                l.chunks.push((b'E', b"Checking a".to_vec()));
            }
            // A fake build has pid 0, which `stop` never signals.
            stop(&b);
            b.log.lock().unwrap().exit = Some(code);
            let b = Arc::new(b);
            reap(&dm, "key", &b);
            assert!(dm.state.lock().unwrap().finished.is_empty(), "exit {code}");
            let frames = client_frames(&b);
            let (kind, why) = frames.last().unwrap();
            assert_eq!(*kind, b'F', "exit {code}");
            assert!(String::from_utf8_lossy(why).contains("stopped"));
            assert!(frames.iter().all(|(k, _)| *k != b'X'));
        }
    }

    #[test]
    fn a_request_waits_for_a_stopped_build_to_exit_and_never_attaches() {
        let dm = Arc::new(bare_daemon());
        let b = Arc::new(fake_build());
        *b.prebuild.lock().unwrap() = true;
        dm.state
            .lock()
            .unwrap()
            .builds
            .insert("key".into(), b.clone());
        // Running and not stopped: nothing to wait for, it stays attachable.
        await_stopped(&dm, "key");
        assert!(dm.state.lock().unwrap().builds.contains_key("key"));
        stop(&b);
        let exits = {
            let b = b.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(150));
                b.log.lock().unwrap().exit = Some(143);
                b.cv.notify_all();
            })
        };
        let t = Instant::now();
        await_stopped(&dm, "key");
        assert!(t.elapsed() >= Duration::from_millis(140));
        assert_eq!(b.log.lock().unwrap().exit, Some(143));
        // Gone before the request looks its key up: it starts its own build.
        assert!(dm.state.lock().unwrap().builds.is_empty());
        exits.join().unwrap();
    }

    #[test]
    fn an_unclaimed_prebuild_ended_by_a_signal_is_never_kept() {
        let dm = bare_daemon();
        for code in SIGNAL_EXITS {
            for early in [false, true] {
                let b = fake_build();
                *b.prebuild.lock().unwrap() = true;
                {
                    let mut l = b.log.lock().unwrap();
                    l.exit = Some(code);
                    l.early_served = early;
                }
                let b = Arc::new(b);
                assert!(no_result(&b, &b.log.lock().unwrap()));
                reap(&dm, "key", &b);
                let st = dm.state.lock().unwrap();
                assert!(st.finished.is_empty(), "exit {code}");
            }
        }
        // A compile error or a pass is a result, and so is the build an
        // agent was attached to when it was interrupted.
        for (code, claimed) in [(0, false), (101, false), (130, true)] {
            let b = fake_build();
            *b.prebuild.lock().unwrap() = true;
            let mut l = b.log.lock().unwrap();
            l.exit = Some(code);
            l.claimed = claimed;
            assert!(!no_result(&b, &l), "exit {code}");
        }
    }

    #[test]
    fn the_prebuild_a_build_waited_for_is_named_to_it() {
        let v = waited_env(Some("20261127-001850999-119917-pre"), 3.5304).unwrap();
        assert_eq!(v, "20261127-001850999-119917-pre 3.530");
        assert_eq!(
            parse_waited(&v),
            Some(("20261127-001850999-119917-pre".into(), 3.53))
        );
        // No run id to name, or nothing a record could trust.
        assert_eq!(waited_env(None, 1.0), None);
        assert_eq!(waited_env(Some("../x"), 1.0), None);
        assert_eq!(waited_env(Some("id"), f64::NAN), None);
        for bad in ["", "id", "id x", "../x 1", "id -1", "id inf", " 1"] {
            assert_eq!(parse_waited(bad), None, "{bad:?}");
        }
        // The note is the daemon's: it never makes two requests differ.
        let plain = req(&[("PATH", "/bin")], &["test"]);
        let noted = req(&[("PATH", "/bin"), (WAITED_ENV, "id 1.000")], &["test"]);
        assert_eq!(request_key(&plain), request_key(&noted));
    }

    #[test]
    fn a_finished_or_running_prebuild_of_these_sources_is_reported_as_reused() {
        let dm = bare_daemon();
        let pre = |stamp: &str| {
            let mut b = fake_build();
            b.stamp = Some(stamp.into());
            *b.prebuild.lock().unwrap() = true;
            *b.compile_only.lock().unwrap() = true;
            b.log.lock().unwrap().exit = Some(0);
            Arc::new(b)
        };
        // Finished before the request: reused, and dropped as before.
        let done = pre("s");
        dm.state
            .lock()
            .unwrap()
            .finished
            .insert("key".into(), done.clone());
        let got = settle_compile_only(&dm, "key", "s").unwrap();
        assert!(Arc::ptr_eq(&got, &done));
        assert!(dm.state.lock().unwrap().finished.is_empty());
        // Of older sources: dropped, and nothing of it is reused.
        dm.state
            .lock()
            .unwrap()
            .finished
            .insert("key".into(), pre("old"));
        assert!(settle_compile_only(&dm, "key", "s").is_none());
        assert!(dm.state.lock().unwrap().finished.is_empty());
        // Still in `builds` when the request arrives: waited for.
        let running = pre("s");
        dm.state
            .lock()
            .unwrap()
            .builds
            .insert("key".into(), running.clone());
        let got = settle_compile_only(&dm, "key", "s").unwrap();
        assert!(Arc::ptr_eq(&got, &running));
        assert!(running.log.lock().unwrap().claimed);
        assert!(dm.state.lock().unwrap().builds.is_empty());
    }

    fn fake_build() -> Build {
        Build {
            stamp: Some("s".into()),
            compile_only: Mutex::default(),
            units_key: Mutex::default(),
            session: String::new(),
            cwd: "/ws".into(),
            run_id: None,
            started: Instant::now(),
            finished: Mutex::default(),
            pid: 0,
            prebuild: Mutex::new(false),
            stopped: Default::default(),
            claim: Mutex::default(),
            log: Mutex::default(),
            cv: Condvar::new(),
        }
    }

    /// Frames a client gets from `stream`, until `X`.
    fn client_frames(b: &Build) -> Vec<(u8, Vec<u8>)> {
        let (mut a, mut c) = UnixStream::pair().unwrap();
        stream(b, &mut a, false, false, 0.0, 0.0).unwrap();
        // A graceful end of stream, held open until the frames are read:
        // closing the socket outright can reset the connection on Windows
        // and discard what the reader has not taken yet.
        a.shutdown(std::net::Shutdown::Write).unwrap();
        let mut v = Vec::new();
        while let Some(m) = read_msg(&mut c).unwrap() {
            v.push(m);
        }
        drop(a);
        v
    }

    #[test]
    fn early_marker_ends_the_first_request_only() {
        let b = fake_build();
        {
            let mut l = b.log.lock().unwrap();
            l.chunks.push((b'E', b"early block\n".to_vec()));
            l.early_at = Some(1);
            l.chunks.push((b'E', b"rustc error\n".to_vec()));
        }
        let first = client_frames(&b);
        assert_eq!(first.len(), 2);
        assert_eq!(first[0].1, b"early block\n");
        let x: Exit = serde_json::from_slice(&first[1].1).unwrap();
        assert!(x.early && x.code == 101);
        assert!(b.log.lock().unwrap().early_served);
        // The next client (an identical rerun) waits for rustc's verdict.
        b.log.lock().unwrap().exit = Some(101);
        let second = client_frames(&b);
        assert_eq!(second.len(), 3);
        let x: Exit = serde_json::from_slice(&second[2].1).unwrap();
        assert!(!x.early && x.code == 101);
    }

    #[test]
    fn finished_build_ignores_its_marker() {
        let b = fake_build();
        {
            let mut l = b.log.lock().unwrap();
            l.chunks.push((b'E', b"early block\n".to_vec()));
            l.early_at = Some(1);
            l.chunks.push((b'E', b"done\n".to_vec()));
            l.exit = Some(0);
        }
        let f = client_frames(&b);
        let x: Exit = serde_json::from_slice(&f.last().unwrap().1).unwrap();
        assert!(!x.early && x.code == 0 && f.len() == 3);
    }

    #[test]
    fn marker_splits_a_chunk() {
        let mut c = b"before\n".to_vec();
        c.extend_from_slice(crate::early::EXIT_MARKER);
        c.extend_from_slice(b"after\n");
        let (a, z) = crate::early::split_exit_marker(&c).unwrap();
        assert_eq!((a, z), (&b"before\n"[..], &b"after\n"[..]));
        assert!(crate::early::split_exit_marker(b"plain").is_none());
    }

    #[test]
    fn frames_round_trip() {
        let mut buf = Vec::new();
        write_msg(&mut buf, b'O', b"hello").unwrap();
        write_msg(&mut buf, b'X', b"{}").unwrap();
        let mut r = &buf[..];
        assert_eq!(read_msg(&mut r).unwrap(), Some((b'O', b"hello".to_vec())));
        assert_eq!(read_msg(&mut r).unwrap(), Some((b'X', b"{}".to_vec())));
        assert_eq!(read_msg(&mut r).unwrap(), None);
    }

    #[test]
    fn disjoint_watch_roots_are_not_dependency_isolation() {
        let t =
            std::env::temp_dir().join(format!("jr-buildd-disjoint-roots-{}", std::process::id()));
        std::fs::create_dir_all(t.join("shared/src")).unwrap();
        std::fs::write(
            t.join("shared/Cargo.toml"),
            "[package]\nname='shared'\nversion='0.1.0'\nedition='2021'\n",
        )
        .unwrap();
        std::fs::write(t.join("shared/src/lib.rs"), "pub fn shared() {}\n").unwrap();
        let workspaces: Vec<_> = ["a", "b"].iter().map(|n| t.join(n)).collect();
        for ws in &workspaces {
            std::fs::create_dir_all(ws.join("member/src")).unwrap();
            std::fs::write(
                ws.join("Cargo.toml"),
                "[workspace]\nmembers=[\"member\"]\nresolver=\"2\"\n",
            )
            .unwrap();
            std::fs::write(
                ws.join("member/Cargo.toml"),
                "[package]\nname='member'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nshared={path=\"../../shared\"}\n",
            )
            .unwrap();
            std::fs::write(ws.join("member/src/lib.rs"), "pub use shared::shared;\n").unwrap();
        }
        let a = source_roots(&workspaces[0]);
        let b = source_roots(&workspaces[1]);
        // The member manifests both resolve the same dependency, but the
        // watch-root heuristic does not follow member manifests.
        assert_eq!(a, vec![workspaces[0].clone()]);
        assert_eq!(b, vec![workspaces[1].clone()]);
        assert!(
            !a.iter()
                .any(|x| b.iter().any(|y| x.starts_with(y) || y.starts_with(x)))
        );
        for ws in &workspaces {
            assert_eq!(
                std::fs::canonicalize(ws.join("member/../../shared")).unwrap(),
                std::fs::canonicalize(t.join("shared")).unwrap()
            );
        }
        let mut st = State::default();
        let r = req(&[("JUSTRUST_BUILDD_NATIVE_EMPTY", "1")], &["check"]);
        st.builds
            .insert("disjoint-workspace".into(), Arc::new(fake_build()));
        assert!(!native_empty_eligible(&st, &r));
        st.finished = std::mem::take(&mut st.builds);
        assert!(!native_empty_eligible(&st, &r));
        std::fs::remove_dir_all(t).unwrap();
    }

    #[test]
    fn stamp_sees_edits_and_path_deps() {
        let t = std::env::temp_dir().join(format!("jr-buildd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let ws = t.join("ws");
        let dep = t.join("dep");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(ws.join("target/debug")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(
            ws.join("Cargo.toml"),
            "[workspace]\n[package]\nname=\"ws\"\n[dependencies]\ndep = { path = \"../dep\" }\n",
        )
        .unwrap();
        std::fs::write(ws.join("src/lib.rs"), "").unwrap();
        std::fs::write(dep.join("Cargo.toml"), "[package]\nname=\"dep\"\n").unwrap();
        std::fs::write(dep.join("src/lib.rs"), "").unwrap();
        let roots = source_roots(&ws);
        assert_eq!(roots, vec![dep.clone(), ws.clone()]);
        let (s0, n) = source_stamp(&roots, &ws, &[]);
        assert_eq!(n, 4);
        assert_eq!(
            source_stamp_with_sort(
                &roots,
                &ws,
                &[],
                |_| {},
                |entries| {
                    entries.sort_by(|a, b| a.0.cmp(&b.0));
                }
            ),
            (s0.clone(), n)
        );
        // Build output does not change the stamp.
        std::fs::write(ws.join("target/debug/x"), "x").unwrap();
        assert_eq!(source_stamp(&roots, &ws, &[]).0, s0);
        // An edit in the path dependency does.
        std::fs::write(dep.join("src/lib.rs"), "pub fn f() {}").unwrap();
        let (s1, _) = source_stamp(&roots, &ws, &[]);
        assert_ne!(s1, s0);
        let _ = std::fs::remove_dir_all(&t);
    }

    /// A workspace with trees its `.justrustignore` keeps out of the mirror:
    /// plain files (`vendor/big`, `data`, `gen/notes.txt`), a path dependency
    /// (`vendor/dep`), a member matched by a pattern (`gen/m`) and a package a
    /// cargo config patches in (`vendor/patched`). 13 files, 3 of them plain.
    fn ignored_trees(name: &str, members: &str) -> (PathBuf, PathBuf, Vec<(String, String)>) {
        let t = std::env::temp_dir().join(format!("jr-buildd-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let ws = t.join("ws");
        let file = |rel: &str, text: &str| {
            let p = ws.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        };
        file(
            "Cargo.toml",
            &format!(
                "[workspace]\nmembers = [{members}]\n[package]\nname = \"ws\"\n[dependencies]\ndep = {{ path = 'vendor/dep' }}\n"
            ),
        );
        file("src/lib.rs", "");
        file(".justrustignore", "/vendor/\n/data/\n/gen/\n");
        file("vendor/big/x.rs", "");
        file("data/a", "");
        file("gen/notes.txt", "");
        file("vendor/dep/Cargo.toml", "[package]\nname = \"dep\"\n");
        file("vendor/dep/src/lib.rs", "");
        file("gen/m/Cargo.toml", "[package]\nname = \"m\"\n");
        file("gen/m/src/lib.rs", "");
        file("vendor/patched/Cargo.toml", "[package]\nname = \"p\"\n");
        file("vendor/patched/src/lib.rs", "");
        file(
            ".cargo/config.toml",
            "[patch.crates-io]\np = { path = \"vendor/patched\" }\n",
        );
        let home = t.join("cargo-home");
        std::fs::create_dir_all(&home).unwrap();
        let env = vec![("CARGO_HOME".to_string(), home.display().to_string())];
        (t, ws, env)
    }

    fn make_mirror(root: &Path) {
        let token = root.join(crate::remote_checkpoint::TOKEN_FILE);
        std::fs::create_dir_all(token.parent().unwrap()).unwrap();
        std::fs::write(token, "token").unwrap();
    }

    #[test]
    fn mirror_stamp_leaves_out_only_what_the_build_cannot_see_change() {
        let (t, ws, env) = ignored_trees("mirror", "\"gen/*\"");
        let roots = source_roots(&ws);
        assert_eq!(roots, vec![ws.clone()]);
        let stamp = || source_stamp(&roots, &ws, &env);
        let edit = |rel: &str| {
            let p = ws.join(rel);
            let mut text = std::fs::read_to_string(&p).unwrap();
            text.push_str("# edited\n");
            std::fs::write(p, text).unwrap();
        };

        // A local checkout: a build reads all of it, so all of it is stamped.
        let (local, n) = stamp();
        assert_eq!(n, 13);
        edit("vendor/big/x.rs");
        let (local_edited, _) = stamp();
        assert_ne!(local_edited, local);

        // The same tree as a mirror: the sync never writes the three plain
        // files, and they are the only ones left out.
        make_mirror(&ws);
        let (s0, n) = stamp();
        assert_eq!(n, 10);
        assert_eq!(
            source_stamp_with_sort(
                &roots,
                &ws,
                &env,
                |_| {},
                |entries| entries.sort_by(|a, b| a.0.cmp(&b.0))
            ),
            (s0.clone(), n)
        );
        for rel in ["vendor/big/x.rs", "data/a", "gen/notes.txt"] {
            edit(rel);
            assert_eq!(stamp().0, s0, "{rel}");
        }
        std::fs::write(ws.join("vendor/big/new.rs"), "").unwrap();
        assert_eq!(stamp().0, s0);

        // Everything the build compiles still changes the stamp, inside an
        // excluded tree or not, and so do the rules themselves.
        let mut last = s0;
        for rel in [
            "src/lib.rs",
            "Cargo.toml",
            "vendor/dep/src/lib.rs",
            "vendor/dep/Cargo.toml",
            "gen/m/src/lib.rs",
            "vendor/patched/src/lib.rs",
            ".cargo/config.toml",
            ".justrustignore",
        ] {
            edit(rel);
            let (next, _) = stamp();
            assert_ne!(next, last, "{rel}");
            last = next;
        }
        std::fs::write(ws.join("vendor/dep/src/new.rs"), "").unwrap();
        assert_ne!(stamp().0, last);

        // A rule that goes away brings its tree back: `data/a` and the file
        // added to the dependency, not `vendor/big`.
        std::fs::write(ws.join(".justrustignore"), "/vendor/\n/gen/\n").unwrap();
        assert_eq!(stamp().1, 12);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn mirror_stamp_prunes_nothing_it_cannot_prove_unused() {
        // A member pattern this does not expand could name any directory.
        let (t, ws, env) = ignored_trees("mirror-glob", "\"gen/**\"");
        make_mirror(&ws);
        let roots = source_roots(&ws);
        assert_eq!(source_stamp(&roots, &ws, &env).1, 13);
        let _ = std::fs::remove_dir_all(&t);

        // Nor does a manifest that does not parse say what it depends on.
        let (t, ws, env) = ignored_trees("mirror-bad", "\"gen/*\"");
        make_mirror(&ws);
        std::fs::write(ws.join("gen/m/Cargo.toml"), "[package\n").unwrap();
        let roots = source_roots(&ws);
        assert_eq!(source_stamp(&roots, &ws, &env).1, 13);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn mirror_rules_of_the_repository_reach_a_workspace_below_it() {
        let t = std::env::temp_dir().join(format!("jr-buildd-below-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&t);
        let (repo, home) = (t.join("repo"), t.join("cargo-home"));
        let ws = repo.join("app");
        let manifest = "[workspace]\n[package]\nname=\"a\"\n";
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::create_dir_all(ws.join("fixtures")).unwrap();
        std::fs::create_dir_all(repo.join("tool/src")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(ws.join("Cargo.toml"), manifest).unwrap();
        std::fs::write(ws.join("src/lib.rs"), "").unwrap();
        std::fs::write(ws.join("fixtures/big"), "").unwrap();
        std::fs::write(repo.join("tool/Cargo.toml"), manifest).unwrap();
        std::fs::write(repo.join("tool/src/lib.rs"), "").unwrap();
        std::fs::write(repo.join(".justrustignore"), "/app/fixtures/\n/tool/\n").unwrap();
        let env = vec![("CARGO_HOME".to_string(), home.display().to_string())];
        let count = |cwd: &Path| source_stamp(&source_roots(cwd), cwd, &env).1;
        assert_eq!(count(&ws), 3);
        make_mirror(&repo);
        assert_eq!(count(&ws), 2);
        // A build started inside an excluded tree reads that tree: the sync
        // mirrors it as a root of its own, and the stamp covers it.
        assert_eq!(count(&repo.join("tool")), 2);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn checkout_whose_builds_are_all_hosted_is_stamped_like_its_mirror() {
        let (t, ws, env) = ignored_trees("local-off", "\"gen/*\"");
        let roots = source_roots(&ws);
        let stamp = || source_stamp(&roots, &ws, &env);
        let edit = |rel: &str| {
            let p = ws.join(rel);
            let mut text = std::fs::read_to_string(&p).unwrap();
            text.push_str("# edited\n");
            std::fs::write(p, text).unwrap();
        };
        LOCAL_OFF.with(|on| on.set(true));
        // Outside git the sync mirrors the whole workspace: all stamped.
        assert_eq!(stamp().1, 13);
        // In a repository the mirror follows `.justrustignore`, and so does
        // the stamp: what no build can read does not change it.
        std::fs::create_dir_all(ws.join(".git")).unwrap();
        let (s0, n) = stamp();
        assert_eq!(n, 10);
        for rel in ["vendor/big/x.rs", "data/a", "gen/notes.txt"] {
            edit(rel);
            assert_eq!(stamp().0, s0, "{rel}");
        }
        // What a hosted build compiles does, inside an excluded tree or not.
        let mut last = s0;
        for rel in [
            "src/lib.rs",
            "vendor/dep/src/lib.rs",
            "gen/m/src/lib.rs",
            "vendor/patched/src/lib.rs",
            ".justrustignore",
        ] {
            edit(rel);
            let (next, _) = stamp();
            assert_ne!(next, last, "{rel}");
            last = next;
        }
        // A machine that may compile here reads all of it again.
        LOCAL_OFF.with(|on| on.set(false));
        assert_eq!(stamp().1, 13);
        let _ = std::fs::remove_dir_all(&t);
    }

    #[test]
    fn member_patterns_expand_within_one_component() {
        assert!(wildcard("*", "a"));
        assert!(wildcard("jr-*-sys", "jr-zlib-sys"));
        assert!(wildcard("a?c", "abc"));
        assert!(!wildcard("a?c", "ac"));
        assert!(!wildcard("jr-*", "other"));
        assert!(wildcard("a*", "a"));
        assert!(expand_member(Path::new("/x"), "crates/**").is_none());
        assert!(expand_member(Path::new("/x"), "crates/[ab]").is_none());
        assert_eq!(
            expand_member(Path::new("/x"), "./crates/../a"),
            Some(vec![PathBuf::from("/x/a")])
        );
    }

    #[test]
    fn exe_replacement_and_deleted_path_are_detected() {
        let dir = std::env::temp_dir().join(format!("jr-buildd-exe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let bin = dir.join("justrust");
        let next = dir.join("justrust.new");
        std::fs::write(&bin, b"v1").unwrap();
        let id = crate::sys::file_id(&std::fs::metadata(&bin).unwrap());
        let initial = (bin.clone(), id);

        assert!(!exe_is_replaced(Some(&initial), Some(&bin)));

        // Linux /proc/self/exe appends " (deleted)" after atomic rename.
        let deleted = PathBuf::from(format!("{} (deleted)", bin.display()));
        assert!(exe_is_replaced(Some(&initial), Some(&deleted)));
        assert_eq!(resolve_live_exe(&deleted), bin);

        // Atomic replacement changes inode on disk even if current_exe isn't suffixed.
        // Without an inode (Windows) only the write time tells them apart: the
        // size is the same and NTFS hands the renamed file the old creation time.
        std::fs::write(&next, b"v2").unwrap();
        let written = std::fs::metadata(&bin).unwrap().modified().unwrap();
        std::fs::File::options()
            .write(true)
            .open(&next)
            .unwrap()
            .set_modified(written + Duration::from_secs(2))
            .unwrap();
        std::fs::rename(&next, &bin).unwrap();
        assert!(exe_is_replaced(Some(&initial), Some(&bin)));

        // Unlinked executable with no replacement is also detected.
        std::fs::remove_file(&bin).unwrap();
        assert!(exe_is_replaced(Some(&initial), Some(&bin)));
        assert_eq!(resolve_live_exe(&deleted), deleted);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prebuild_selection_enforces_loop_cap_and_backoff() {
        let mut st = State::default();
        let now = Instant::now();
        let root = PathBuf::from("/ws");
        let dirty = vec![PathBuf::from("/ws/src/lib.rs")];
        let queued: Vec<String> = Vec::new();

        let make_req = |session: &str, argv: &[&str]| Request {
            build: String::new(),
            cwd: "/ws".into(),
            argv: argv.iter().map(|s| (*s).to_owned()).collect(),
            env: vec![("JUSTRUST_SLOT_SESSION".into(), session.into())],
            prebuild: false,
        };

        // 1. Loop detection: first request (seen == 1) does not trigger prebuild.
        let r_check = make_req("s1", &["check"]);
        let k_check = request_key(&r_check);
        remember_request(&mut st, &r_check, &k_check, std::slice::from_ref(&root));
        assert!(select_prebuild_candidates(&st, &dirty, &[], &queued, now).is_empty());

        // Second request for the same command establishes a loop (seen == 2).
        remember_request(&mut st, &r_check, &k_check, std::slice::from_ref(&root));
        let got = select_prebuild_candidates(&st, &dirty, &[], &queued, now);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, k_check);

        // 2. Single-prebuild cap per workspace: two eligible commands in /ws -> only most recent.
        let r_test = make_req("s2", &["test", "--lib"]);
        let k_test = request_key(&r_test);
        remember_request(&mut st, &r_test, &k_test, std::slice::from_ref(&root));
        remember_request(&mut st, &r_test, &k_test, std::slice::from_ref(&root));
        st.recent.get_mut(&k_check).unwrap().used = now - Duration::from_secs(10);
        st.recent.get_mut(&k_test).unwrap().used = now;
        let got = select_prebuild_candidates(&st, &dirty, &[], &queued, now);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, k_test);

        // In-flight build in /ws blocks any prebuild for /ws.
        st.builds.insert("active".into(), Arc::new(fake_build()));
        assert!(select_prebuild_candidates(&st, &dirty, &[], &queued, now).is_empty());
        st.builds.clear();

        // 3. Adaptive backoff: miss puts workspace in cooldown; hit clears it.
        record_prebuild_miss(&mut st, "/ws", now);
        assert!(in_prebuild_cooldown(
            &st,
            "/ws",
            now + Duration::from_secs(5)
        ));
        assert!(
            select_prebuild_candidates(&st, &dirty, &[], &queued, now + Duration::from_secs(5))
                .is_empty()
        );
        assert!(!in_prebuild_cooldown(
            &st,
            "/ws",
            now + Duration::from_secs(20)
        ));
        record_prebuild_hit(&mut st, "/ws");
        assert!(!in_prebuild_cooldown(&st, "/ws", now));
        assert_eq!(
            select_prebuild_candidates(&st, &dirty, &[], &queued, now).len(),
            1
        );
    }
}
