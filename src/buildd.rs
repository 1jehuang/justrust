//! The resident build daemon: `justrust __buildd`.
//!
//! One per user, started on demand by the first agent build that wants it,
//! exits after 15 minutes without requests or running builds. Agent builds
//! (`justrust check|test|build|clippy`) hand their request to it over
//! `~/.justrust/buildd/sock` instead of running cargo themselves:
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
//! `JUSTRUST_BUILDD_PREBUILD=0` only prebuilds (FINDINGS.md, build daemon).
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

pub const DAEMON_ARG: &str = "__buildd";
/// Set on the builds the daemon runs: they never go back to the daemon.
const CHILD_ENV: &str = "JUSTRUST_BUILDD_CHILD";
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
    // Benchmarks pin run ids; an attached request links its id to the
    // shared run instead (`Exit::run_id`).
    "JUSTRUST_RUN_ID_OVERRIDE",
];
const IGNORED_ENV_PREFIXES: &[&str] = &["KITTY_", "JCODE_SPAWN_", "JCODE_TUI_", "JCODE_RELOAD_"];

fn env_ignored(name: &str) -> bool {
    IGNORED_ENV.contains(&name) || IGNORED_ENV_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// On by default; `JUSTRUST_BUILDD=0` builds in the client as before.
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

pub fn sock_path() -> Result<PathBuf> {
    Ok(dir()?.join("sock"))
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
        || std::env::var_os(crate::route::ON_AGENT).is_some()
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
                if x.replayed {
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

fn spawn() -> Result<()> {
    let d = dir()?;
    std::fs::create_dir_all(&d)?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(d.join("daemon.log"))?;
    let mut c = Command::new(std::env::current_exe()?);
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

/// Hash of the source state under `roots`: every file's path, size, mtime
/// and inode, skipping build output and hidden directories (except
/// `.cargo`, whose config changes builds), plus cargo config files in the
/// ancestors of `cwd` and in CARGO_HOME.
pub fn source_stamp(roots: &[PathBuf], cwd: &Path, env: &[(String, String)]) -> (String, usize) {
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
    for r in roots {
        let mut stack = vec![r.clone()];
        let mut entries: Vec<(PathBuf, std::fs::Metadata)> = Vec::new();
        while let Some(d) = stack.pop() {
            let Ok(rd) = std::fs::read_dir(&d) else {
                continue;
            };
            for e in rd.flatten() {
                let name = e.file_name();
                let n = name.to_string_lossy();
                let Ok(ft) = e.file_type() else { continue };
                if ft.is_dir() {
                    if n == "target"
                        || n.starts_with("target-")
                        || (n.starts_with('.') && n != ".cargo")
                    {
                        continue;
                    }
                    stack.push(e.path());
                } else if let Ok(m) = std::fs::metadata(e.path()) {
                    entries.push((e.path(), m));
                }
            }
        }
        entries.sort_by(|a, b| a.0.cmp(&b.0));
        files += entries.len();
        for (p, m) in &entries {
            add(p, m);
        }
    }
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
    for c in configs {
        if let Ok(m) = std::fs::metadata(&c) {
            add(&c, &m);
        }
    }
    (h.hex(), files)
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
}

struct Build {
    stamp: String,
    /// A `test` prebuilt as `test --no-run`: requests wait for it, then run
    /// for real (on Fresh units), never take its output.
    compile_only: Mutex<bool>,
    /// Agent session that asked (`JUSTRUST_SLOT_SESSION`).
    session: String,
    run_id: Option<String>,
    started: Instant,
    finished: Mutex<Option<Instant>>,
    pid: i32,
    prebuild: Mutex<bool>,
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
    served: u64,
    coalesced: u64,
    prebuilt: u64,
    last_request: Option<Instant>,
}

/// A request to prebuild when its sources change (`JUSTRUST_BUILDD_PREBUILD=0` disables).
struct Recent {
    req: Request,
    roots: Vec<PathBuf>,
    used: Instant,
    /// Prebuilds left until the agent asks again. Bounds the loop of a
    /// build that writes into its own sources (snapshot tests, generated
    /// files): each of its writes would otherwise start another prebuild.
    budget: u8,
}

const PREBUILDS_PER_REQUEST: u8 = 3;

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
    draining: std::sync::atomic::AtomicBool,
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
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(d.join("lock"))?;
    if lock.try_lock().is_err() {
        bail!("another build daemon is running");
    }
    let sock = sock_path()?;
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    let md = std::fs::metadata(&sock)?;
    log(&format!(
        "build daemon up, build {}",
        crate::remote_daemon::build_id()
    ));
    let (wtx, wrx) = std::sync::mpsc::channel::<PathBuf>();
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
        draining: Default::default(),
    });
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
            dm.draining.store(true, std::sync::atomic::Ordering::SeqCst);
            remove_sock(dm);
            write_msg(&mut s, b'I', &serde_json::to_vec(&info(dm, false))?)?;
            log("shutdown requested, draining");
            let dm = dm.clone();
            std::thread::spawn(move || {
                while !dm.state.lock().unwrap().builds.is_empty() {
                    std::thread::sleep(Duration::from_millis(100));
                }
                std::process::exit(0);
            });
            Ok(())
        }
        b'R' => {
            let req: Request = serde_json::from_slice(&body)?;
            if req.build != crate::remote_daemon::build_id() && !req.build.is_empty() {
                // Another justrust build: hand back and step aside, so the
                // client's next request starts a daemon of its own build.
                write_msg(&mut s, b'F', b"build daemon is another justrust build")?;
                dm.draining.store(true, std::sync::atomic::Ordering::SeqCst);
                remove_sock(dm);
                log("replaced by another justrust build, draining");
                let dm = dm.clone();
                std::thread::spawn(move || {
                    while !dm.state.lock().unwrap().builds.is_empty() {
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    std::process::exit(0);
                });
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
    let (stamp, _files) = source_stamp(&roots, &cwd, &req.env);
    if !req.prebuild {
        cancel_stale_prebuild(dm, &key, &stamp);
        cancel_wrong_guess(dm, &key, &session_of(&req));
        settle_compile_only(dm, &key, &stamp);
    }
    let waited_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let (build, attached, replayed) = {
        let mut st = dm.state.lock().unwrap();
        st.served += 1;
        st.last_request = Some(Instant::now());
        let running = st
            .builds
            .get(&key)
            .filter(|b| {
                b.stamp == stamp
                    && b.log.lock().unwrap().exit.is_none()
                    && !*b.compile_only.lock().unwrap()
            })
            .cloned();
        // A prebuild of exactly this request finished before it arrived.
        let done = if req.prebuild || running.is_some() {
            None
        } else {
            st.finished.remove(&key).filter(|b| {
                b.stamp == stamp
                    && b.started.elapsed() < FINISHED_MAX_AGE
                    && !*b.compile_only.lock().unwrap()
            })
        };
        if !req.prebuild && wants_prebuild(&req) {
            st.recent.insert(
                key.clone(),
                Recent {
                    req: req.clone(),
                    roots: roots.clone(),
                    used: Instant::now(),
                    budget: PREBUILDS_PER_REQUEST,
                },
            );
            prune_recent(&mut st.recent);
        }
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
                let b = start(&req, stamp, &cwd)?;
                st.builds.insert(key.clone(), b.clone());
                let (dm2, key2, b2) = (dm.clone(), key.clone(), b.clone());
                std::thread::spawn(move || reap(&dm2, &key2, &b2));
                (b, false, false)
            }
        }
    };
    let lookup_ms = t0.elapsed().as_secs_f64() * 1000.0;
    if !req.prebuild && wants_prebuild(&req) {
        watch_roots(dm, &roots);
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
        write_msg(&mut s, b'I', &serde_json::to_vec(&info(dm, attached))?)?;
        return Ok(());
    }
    log(&format!(
        "{} {} in {} ({:.1} ms, {:.0} ms waiting for a compile-only prebuild, stamp {})",
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
        waited_ms,
        &build.stamp[..8]
    ));
    let res = stream(&build, &mut s, attached, replayed, joined_after, lookup_ms);
    let mut l = build.log.lock().unwrap();
    l.clients -= 1;
    let orphaned = l.clients == 0 && l.exit.is_none() && !*build.prebuild.lock().unwrap();
    drop(l);
    if orphaned {
        // Everyone who asked is gone (killed or timed out): stop the build
        // like their Ctrl+C or kill would have without the daemon.
        log(&format!("no clients left, stopping pid {}", build.pid));
        crate::sys::terminate_group(build.pid, false);
    }
    res
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
        let (batch, exit, sharers) = {
            let mut l = build.log.lock().unwrap();
            while next >= l.chunks.len() && l.exit.is_none() {
                // A client killed while the build is quiet is noticed
                // within a second, not at the next output.
                let (g, t) = build.cv.wait_timeout(l, Duration::from_secs(1)).unwrap();
                l = g;
                if t.timed_out() && peer_closed(s) {
                    bail!("client went away");
                }
            }
            let batch: Vec<(u8, Vec<u8>)> = l.chunks[next..].to_vec();
            next = l.chunks.len();
            (
                batch,
                if next >= l.chunks.len() { l.exit } else { None },
                l.sharers,
            )
        };
        for (k, b) in &batch {
            write_msg(s, *k, b)?;
        }
        if let Some(code) = exit {
            let x = Exit {
                code,
                attached,
                joined_after,
                sharers,
                lookup_ms,
                run_id: build.run_id.clone(),
                replayed,
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

fn start(req: &Request, stamp: String, cwd: &Path) -> Result<Arc<Build>> {
    let mut c = Command::new(std::env::current_exe()?);
    c.args(&req.argv)
        .current_dir(cwd)
        .env_clear()
        .envs(req.env.iter().map(|(k, v)| (k, v)))
        .env(CHILD_ENV, "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    c.detached();
    if req.prebuild {
        // A guess: below every build an agent is waiting for.
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            // SAFETY: setpriority is async-signal-safe.
            unsafe {
                c.pre_exec(|| {
                    libc::setpriority(libc::PRIO_PROCESS, 0, 10);
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
        session: session_of(req),
        run_id: req
            .env
            .iter()
            .find(|(k, _)| k == "JUSTRUST_RUN_ID_OVERRIDE")
            .map(|(_, v)| v.clone()),
        started: Instant::now(),
        finished: Mutex::default(),
        pid: child.id() as i32,
        prebuild: Mutex::new(req.prebuild),
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
                        b.log.lock().unwrap().chunks.push((k, buf[..n].to_vec()));
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
                b.stamp != stamp && *b.prebuild.lock().unwrap() && !b.log.lock().unwrap().claimed
            })
            .cloned()
    };
    let Some(b) = stale else { return };
    log(&format!("sources changed, stopping prebuild pid {}", b.pid));
    crate::sys::terminate_group(b.pid, false);
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

fn is_test(argv: &[String]) -> bool {
    matches!(argv.first().map(String::as_str), Some("test" | "t"))
}

/// `test ARGS -- FILTERS` -> `test ARGS --no-run`: the same units, no run.
fn no_run_argv(argv: &[String]) -> Vec<String> {
    let mut out: Vec<String> = argv.iter().take_while(|a| *a != "--").cloned().collect();
    if !out.iter().any(|a| a == "--no-run") {
        out.push("--no-run".into());
    }
    out
}

/// A compile-only prebuild of this request and these sources is running:
/// wait for it (the agent's build would wait on its slot anyway), so the
/// real run that follows finds every unit Fresh. A finished one is dropped.
fn settle_compile_only(dm: &Daemon, key: &str, stamp: &str) {
    let b = {
        let mut st = dm.state.lock().unwrap();
        if st
            .finished
            .get(key)
            .is_some_and(|b| *b.compile_only.lock().unwrap())
        {
            st.finished.remove(key);
        }
        st.builds
            .get(key)
            .filter(|b| b.stamp == stamp && *b.compile_only.lock().unwrap())
            .cloned()
    };
    let Some(b) = b else { return };
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
            .filter(|(k, b)| {
                k.as_str() != key
                    && b.session == session
                    && *b.prebuild.lock().unwrap()
                    && !b.log.lock().unwrap().claimed
            })
            .map(|(_, b)| b.clone())
            .collect()
    };
    for b in wrong {
        log(&format!(
            "agent asked for another build, stopping prebuild pid {}",
            b.pid
        ));
        crate::sys::terminate_group(b.pid, false);
    }
}

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
        let wanted: Vec<(String, Request, Vec<PathBuf>)> = {
            let mut st = dm.state.lock().unwrap();
            prune_recent(&mut st.recent);
            st.finished
                .retain(|_, b| b.started.elapsed() < FINISHED_MAX_AGE);
            st.recent
                .iter()
                .filter(|(k, r)| {
                    r.budget > 0
                        && (retry.contains(k)
                            || dirty
                                .iter()
                                .any(|d| r.roots.iter().any(|x| d.starts_with(x))))
                })
                .map(|(k, r)| (k.clone(), r.req.clone(), r.roots.clone()))
                .collect()
        };
        dirty.clear();
        retry.clear();
        for (key, mut req, roots) in wanted {
            let cwd = PathBuf::from(&req.cwd);
            let (stamp, _) = source_stamp(&roots, &cwd, &req.env);
            // An unclaimed prebuild of older sources is useless now.
            cancel_stale_prebuild(dm, &key, &stamp);
            let mut st = dm.state.lock().unwrap();
            if let Some(b) = st.builds.get(&key) {
                if b.stamp != stamp {
                    retry.push(key);
                }
                continue;
            }
            if st.finished.get(&key).is_some_and(|b| b.stamp == stamp) {
                continue;
            }
            req.prebuild = true;
            // Never run an agent's tests on a guess: they may touch files,
            // sockets or processes, and a cancelled prebuild would kill
            // them mid-run. Compile the same units only.
            let compile_only = is_test(&req.argv);
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
            match start(&req, stamp, &cwd) {
                Ok(b) => {
                    *b.compile_only.lock().unwrap() = compile_only;
                    log(&format!(
                        "prebuild {} in {} (stamp {})",
                        req.argv.join(" "),
                        req.cwd,
                        &b.stamp[..8]
                    ));
                    if let Some(r) = st.recent.get_mut(&key) {
                        r.budget = r.budget.saturating_sub(1);
                    }
                    st.builds.insert(key.clone(), b.clone());
                    let (dm2, b2) = (dm.clone(), b.clone());
                    std::thread::spawn(move || reap(&dm2, &key, &b2));
                }
                Err(e) => log(&format!("prebuild failed to start: {e:#}")),
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
    let mut st = dm.state.lock().unwrap();
    if st.builds.get(key).is_some_and(|x| Arc::ptr_eq(x, b)) {
        st.builds.remove(key);
    }
    // Nobody saw it: keep it for the request it was started for.
    if !b.log.lock().unwrap().claimed && *b.prebuild.lock().unwrap() {
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
        assert!(is_test(&v(&["t"])) && !is_test(&v(&["check"])));
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
        // Build output does not change the stamp.
        std::fs::write(ws.join("target/debug/x"), "x").unwrap();
        assert_eq!(source_stamp(&roots, &ws, &[]).0, s0);
        // An edit in the path dependency does.
        std::fs::write(dep.join("src/lib.rs"), "pub fn f() {}").unwrap();
        let (s1, _) = source_stamp(&roots, &ws, &[]);
        assert_ne!(s1, s0);
        let _ = std::fs::remove_dir_all(&t);
    }
}
