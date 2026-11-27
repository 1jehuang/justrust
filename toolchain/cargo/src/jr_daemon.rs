//! justrust: jr-cargo fork server (default on, `JUSTRUST_CARGO_DAEMON=0` opts out).
//!
//! A warm `cargo check`/`test` on a big workspace spends 150-250 ms before
//! the first rustc starts: loading manifests and config, resolving, reading
//! index summaries, building the unit graph (`create_bcx`). None of that
//! changes while an agent edits `.rs` files. This module keeps a parked
//! process per exact invocation that has already done that work, and forks
//! it per request: the fork is a copy-on-write snapshot right after
//! `create_bcx`, so each request starts at the fingerprint/compile stage.
//!
//! Protocol (Unix socket `$XDG_RUNTIME_DIR/jr-cargo/<key>.sock`, `key` =
//! hash of the cargo binary, argv, cwd and the environment minus variables
//! that only identify the caller, see [`volatile`]):
//!
//! - The client (the `cargo` the user ran) sends its stdin/stdout/stderr
//!   (SCM_RIGHTS) and its full environment.
//! - The server recomputes its stamp (stat of every manifest, `Cargo.lock`,
//!   cargo config file and rustc, plus the entry names of every directory
//!   that target auto-discovery reads). Mismatch: it answers `S` and exits,
//!   and the client runs normally. Otherwise it forks; the child answers
//!   `R`, takes over the client's fds and environment, runs the rest of the
//!   build, and sends the exit code (4 bytes) before exiting.
//! - No server, a stale one, or any error before `R`: the client runs as
//!   plain cargo (fail open) and, when it exits, spawns a fresh server
//!   (same argv and environment, `JUSTRUST_CARGO_SERVE=<socket>`), which
//!   runs up to `create_bcx` and parks there.
//!
//! Only `build`/`check`/`test` with stdout and stderr not on a terminal and
//! no inherited jobserver use it. Known differences from a plain run: the
//! daemon child skips cargo's once-a-day auto-gc of `CARGO_HOME`, and the
//! config environment snapshot keeps the server's values for the volatile
//! variables (justrust run ids; they never reach cargo's build decisions).
//! A server exits after 15 minutes without requests, when it is not among
//! the [`MAX_PARKED`] most recently used servers (socket mtime, touched per
//! request), or when `MemAvailable` drops below [`MEM_FLOOR_KB`]. Each
//! parked server costs ~230 MB RSS (FINDINGS 61).

#![allow(
    clippy::disallowed_methods,
    reason = "justrust: the client runs before any GlobalContext exists"
)]

use crate::compiler::BuildContext;
use crate::util::StableHasher;
use std::ffi::OsString;
use std::hash::{Hash, Hasher};
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStringExt;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::time::{Duration, Instant, SystemTime};

const ENABLE_ENV: &str = "JUSTRUST_CARGO_DAEMON";
const SERVE_ENV: &str = "JUSTRUST_CARGO_SERVE";
const PROTOCOL: u32 = 1;
const IDLE: Duration = Duration::from_secs(15 * 60);
const TICK: Duration = Duration::from_secs(20);
const MAX_PARKED: usize = 6;
const MEM_FLOOR_KB: u64 = 2 * 1024 * 1024;
/// [`sweep`] leaves files younger than this alone: a server that is still
/// starting has logs and a start marker but no pid file yet.
const SWEEP_GRACE: Duration = Duration::from_secs(120);
/// A socket without a pid file (a server from before pid files) is dead once
/// it is this old: a live server touches its socket per request and exits
/// after [`IDLE`].
const SWEEP_UNOWNED_SOCK: Duration = Duration::from_secs(60 * 60);
const SWEEP_DAY: Duration = Duration::from_secs(24 * 60 * 60);

/// Variables that identify the caller (agent, terminal, justrust run), not
/// the build. Excluded from the key; the child still gets the client's
/// exact values.
fn volatile(k: &str) -> bool {
    const EXACT: &[&str] = &[
        "JUSTRUST_RUN_ID",
        "JUSTRUST_RUN_DIR",
        "JUSTRUST_RUN_ID_OVERRIDE",
        "JUSTRUST_TEST_POOL",
        "JUSTRUST_SLOT_SESSION",
        SERVE_ENV,
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
    ];
    const PREFIX: &[&str] = &["KITTY_", "JCODE_SPAWN_", "JCODE_TUI_", "JCODE_RELOAD_"];
    EXACT.contains(&k) || PREFIX.iter().any(|p| k.starts_with(p))
}

/// Client fell back to a normal run: spawn a server for this socket at exit.
static SPAWN_AT_EXIT: OnceLock<PathBuf> = OnceLock::new();
/// This process is a server (`JUSTRUST_CARGO_SERVE`) started at this time.
static SERVING: OnceLock<(PathBuf, SystemTime)> = OnceLock::new();
/// Forked child: the connection to report the exit code on.
static CHILD_CONN: AtomicI32 = AtomicI32::new(-1);
static CHILD_START: OnceLock<Instant> = OnceLock::new();
static PARKED: AtomicBool = AtomicBool::new(false);
/// The child reported its exit code: the client may hang up now.
static FINISHED: AtomicBool = AtomicBool::new(false);

/// Forked child: the client's environment (the GlobalContext snapshot was
/// taken in the server, whose volatile variables differ).
static CHILD_ENV: OnceLock<crate::util::data_structures::HashMap<OsString, OsString>> =
    OnceLock::new();

/// In a forked child, the client's value of a volatile variable (`Some(None)`
/// = unset for the client). `None`: use the snapshot.
pub fn child_env(key: &std::ffi::OsStr) -> Option<Option<&'static std::ffi::OsStr>> {
    let env = CHILD_ENV.get()?;
    if !key.to_str().is_some_and(volatile) {
        return None;
    }
    Some(env.get(key).map(|v| v.as_os_str()))
}

/// This process is a forked daemon child running a client's build.
pub fn is_child() -> bool {
    CHILD_CONN.load(Ordering::Relaxed) >= 0
}

/// A server that is still starting is about to do `what`, which a user
/// would see or wait for (a network update, a rewritten `Cargo.lock`). That
/// belongs to the user's own next command, with its messages: the server
/// exits instead. A no-op in every other process, including a server's
/// forked child, which is a user's command.
pub fn server_must_not(what: &str) {
    let Some((sock, _)) = SERVING.get() else {
        return;
    };
    if is_child() || PARKED.load(Ordering::SeqCst) {
        return;
    }
    eprintln!("jr-cargo daemon: not parking: a server does not {what}");
    let _ = std::fs::remove_file(starting_marker(sock));
    std::process::exit(0);
}

/// Start of the client's request, for "Finished in" and timings.
pub fn child_start() -> Option<Instant> {
    CHILD_START.get().copied()
}

fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(d) if !d.is_empty() => PathBuf::from(d).join("jr-cargo"),
        _ => std::env::temp_dir().join(format!("jr-cargo-{}", unsafe { libc::getuid() })),
    }
}

fn subcommand(args: &[OsString]) -> Option<&str> {
    let mut it = args.iter().skip(1);
    while let Some(a) = it.next() {
        let a = a.to_str()?;
        if matches!(a, "--config" | "-Z" | "-C" | "--color" | "--explain") {
            it.next();
            continue;
        }
        if a.starts_with('+') || a.starts_with('-') {
            continue;
        }
        return Some(a);
    }
    None
}

/// Whether fork servers may be running for this user's builds.
pub fn enabled() -> bool {
    !std::env::var_os(ENABLE_ENV).is_some_and(|v| v == "0")
}

/// The socket for this invocation, if it may use the daemon.
fn eligible_socket() -> Option<PathBuf> {
    if std::env::var_os(ENABLE_ENV).is_some_and(|v| v == "0")
        || std::env::var_os(SERVE_ENV).is_some()
    {
        return None;
    }
    for v in ["MAKEFLAGS", "MFLAGS", "CARGO_MAKEFLAGS", "__CARGO_FIX_PLZ"] {
        if std::env::var_os(v).is_some() {
            return None;
        }
    }
    if unsafe { libc::isatty(1) == 1 || libc::isatty(2) == 1 } {
        return None;
    }
    let args: Vec<OsString> = std::env::args_os().collect();
    if !matches!(
        subcommand(&args)?,
        "build" | "b" | "check" | "c" | "test" | "t"
    ) {
        return None;
    }
    let exe = std::env::current_exe().ok()?;
    let m = std::fs::metadata(&exe).ok()?;
    let cwd = std::env::current_dir().ok()?;
    let mut env: Vec<(OsString, OsString)> = std::env::vars_os()
        .filter(|(k, _)| k.to_str().is_none_or(|k| !volatile(k)))
        .collect();
    env.sort();
    let mut h = StableHasher::new();
    PROTOCOL.hash(&mut h);
    (&exe, m.dev(), m.ino(), m.mtime(), m.mtime_nsec(), m.size()).hash(&mut h);
    (args, cwd, env, unsafe { libc::getuid() }).hash(&mut h);
    let key = Hasher::finish(&h);
    Some(runtime_dir().join(format!("{key:016x}.sock")))
}

/// Called first thing in `main`. Serves this invocation through a parked
/// server when one matches (never returns then), else returns and cargo
/// runs normally.
pub fn client_main() {
    if let Some(sock) = std::env::var_os(SERVE_ENV) {
        let _ = SERVING.set((PathBuf::from(sock), SystemTime::now()));
        return;
    }
    let Some(sock) = eligible_socket() else {
        return;
    };
    match request(&sock) {
        Ok(Reply::Done(code)) => std::process::exit(code),
        Ok(Reply::Lost) => {
            eprintln!("error: jr-cargo daemon: the build process died without an exit code");
            std::process::exit(101)
        }
        Ok(Reply::Fallback) | Err(_) => {
            if !nopark_marker(&sock).exists() {
                let _ = SPAWN_AT_EXIT.set(sock);
            }
        }
    }
}

enum Reply {
    Fallback,
    Done(i32),
    Lost,
}

fn request(sock: &Path) -> io::Result<Reply> {
    let mut s = match UnixStream::connect(sock) {
        Ok(s) => s,
        Err(e) => {
            if e.kind() == io::ErrorKind::ConnectionRefused {
                let _ = std::fs::remove_file(sock);
            }
            return Err(e);
        }
    };
    let mut payload = Vec::new();
    for (k, v) in std::env::vars_os() {
        payload.extend_from_slice(&k.into_vec());
        payload.push(b'=');
        payload.extend_from_slice(&v.into_vec());
        payload.push(0);
    }
    let mut msg = (payload.len() as u32).to_le_bytes().to_vec();
    msg.extend_from_slice(&payload);
    send_with_fds(&s, &msg, &[0, 1, 2])?;
    let mut b = [0u8; 1];
    if s.read(&mut b)? == 0 || b[0] != b'R' {
        return Ok(Reply::Fallback);
    }
    let mut code = [0u8; 4];
    Ok(match s.read_exact(&mut code) {
        Ok(()) => Reply::Done(i32::from_le_bytes(code)),
        Err(_) => Reply::Lost,
    })
}

fn nopark_marker(sock: &Path) -> PathBuf {
    sock.with_extension("nopark")
}

/// Pids holding an flock on the file with inode `ino`, from the text of
/// `/proc/locks` (`1: FLOCK  ADVISORY  WRITE 1234 fd:01:5678 0 EOF`; lines
/// with `->` are waiters, not holders).
fn flock_holders(locks: &str, ino: u64) -> Vec<i32> {
    let mut pids = Vec::new();
    for line in locks.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 6 || f[1] != "FLOCK" {
            continue;
        }
        let holds = f[5].rsplit(':').next().and_then(|i| i.parse().ok()) == Some(ino);
        if let (true, Ok(pid)) = (holds, f[4].parse::<i32>()) {
            if pid > 0 && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }
    pids
}

/// A cargo blocked on the lock file `path`. Kills every holder that is a
/// jr-cargo server (it has `JUSTRUST_CARGO_SERVE` in its environment; a
/// parked server holds no cache lock, so a holder is one still starting)
/// and waits for the kernel to release its locks. Returns whether any was
/// stopped. A server killed here did no build and is respawned by the next
/// client that falls back; cargo's cache tolerates a killed cargo.
pub fn stop_starting_servers_holding(path: &Path) -> bool {
    if SERVING.get().is_some() || is_child() {
        return false;
    }
    let (Ok(meta), Ok(locks)) = (
        std::fs::metadata(path),
        std::fs::read_to_string("/proc/locks"),
    ) else {
        return false;
    };
    let serve = format!("{SERVE_ENV}=");
    let trace = std::env::var_os("JUSTRUST_CARGO_DAEMON_TRACE").is_some();
    let mut stopped = false;
    for pid in flock_holders(&locks, meta.ino()) {
        let Ok(env) = std::fs::read(format!("/proc/{pid}/environ")) else {
            if trace {
                eprintln!("jr-cargo daemon: holder {pid}: environment unreadable");
            }
            continue;
        };
        if trace {
            let cmd = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
            eprintln!(
                "jr-cargo daemon: {} is held by pid {pid}: {}",
                path.display(),
                String::from_utf8_lossy(&cmd).replace('\0', " ")
            );
        }
        let Some(sock) = env
            .split(|b| *b == 0)
            .find_map(|v| v.strip_prefix(serve.as_bytes()))
        else {
            continue;
        };
        if unsafe { libc::kill(pid, libc::SIGKILL) } != 0 {
            continue;
        }
        stopped = true;
        // Its start attempt is over: let the next client spawn a new one.
        let sock = PathBuf::from(std::ffi::OsString::from_vec(sock.to_vec()));
        let _ = std::fs::remove_file(starting_marker(&sock));
        let gone = Instant::now() + Duration::from_millis(500);
        while Path::new(&format!("/proc/{pid}")).exists() && Instant::now() < gone {
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    stopped
}

fn starting_marker(sock: &Path) -> PathBuf {
    sock.with_extension("starting")
}

/// Called right before cargo exits with `code` (also from the normal
/// return of `main`).
pub fn finish(code: i32) {
    let fd = CHILD_CONN.load(Ordering::Relaxed);
    if fd >= 0 {
        let _ = io::stdout().flush();
        let _ = io::stderr().flush();
        FINISHED.store(true, Ordering::SeqCst);
        let bytes = code.to_le_bytes();
        unsafe { libc::write(fd, bytes.as_ptr().cast(), 4) };
        return;
    }
    if let Some(sock) = SPAWN_AT_EXIT.get() {
        let _ = spawn_server(sock);
    }
}

fn spawn_server(sock: &Path) -> io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::{Command, Stdio};
    let dir = sock.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    std::fs::set_permissions(dir, std::os::unix::fs::PermissionsExt::from_mode(0o700))?;
    // One server start at a time per socket (a start takes ~0.3-1 s).
    let marker = starting_marker(sock);
    if let Ok(m) = std::fs::metadata(&marker)
        && m.modified()
            .is_ok_and(|t| t.elapsed().unwrap_or_default() < Duration::from_secs(30))
    {
        return Ok(());
    }
    std::fs::write(&marker, b"")?;
    let log = |ext: &str| std::fs::File::create(sock.with_extension(ext));
    let mut cmd = Command::new(std::env::current_exe()?);
    cmd.args(std::env::args_os().skip(1))
        .env(SERVE_ENV, sock)
        .stdin(Stdio::null())
        .stdout(log("out")?)
        .stderr(log("err")?);
    for k in [
        "JUSTRUST_RUN_ID",
        "JUSTRUST_RUN_DIR",
        "JUSTRUST_RUN_ID_OVERRIDE",
        "JUSTRUST_TEST_POOL",
    ] {
        cmd.env_remove(k);
    }
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            // Inherited fds (a caller's flock, pipes it waits on) would be
            // held for the server's whole life.
            libc::close_range(3, u32::MAX, 0);
            Ok(())
        });
    }
    cmd.spawn()?;
    Ok(())
}

/// What the parked state depends on beyond argv and environment.
fn stamp(bcx: &BuildContext<'_, '_>) -> (u64, Option<SystemTime>) {
    let ws = bcx.ws;
    let gctx = bcx.gctx;
    let mut files: Vec<PathBuf> = vec![
        ws.lock_root().as_path_unlocked().join("Cargo.lock"),
        ws.root_manifest().to_path_buf(),
        bcx.rustc().path.clone(),
    ];
    files.extend(bcx.rustc().wrapper.clone());
    files.extend(bcx.rustc().workspace_wrapper.clone());
    if let Some(r) = std::env::var_os("JUSTRUST_REAL_RUSTC") {
        files.push(PathBuf::from(r));
    }
    for dir in gctx.cwd().ancestors() {
        files.push(dir.join(".cargo/config.toml"));
        files.push(dir.join(".cargo/config"));
    }
    let home = gctx.home().as_path_unlocked();
    files.push(home.join("config.toml"));
    files.push(home.join("config"));
    let mut roots: Vec<PathBuf> = ws.members().map(|p| p.root().to_path_buf()).collect();
    roots.extend(
        bcx.packages
            .package_ids()
            .filter_map(|id| id.source_id().local_path()),
    );
    roots.sort();
    roots.dedup();
    let mut dirs = Vec::new();
    for r in &roots {
        files.push(r.join("Cargo.toml"));
        // Workspace roots a path package inherits from.
        for anc in r.ancestors().skip(1) {
            let m = anc.join("Cargo.toml");
            if m.is_file() {
                files.push(m);
                break;
            }
        }
        dirs.push(r.clone());
        dirs.push(r.join("src"));
        for sub in ["src/bin", "tests", "examples", "benches"] {
            let d = r.join(sub);
            if let Ok(rd) = std::fs::read_dir(&d) {
                for e in rd.flatten() {
                    if e.file_type().is_ok_and(|t| t.is_dir()) {
                        dirs.push(e.path());
                    }
                }
            }
            dirs.push(d);
        }
    }
    files.sort();
    files.dedup();
    dirs.sort();
    dirs.dedup();
    let mut h = StableHasher::new();
    let mut newest: Option<SystemTime> = None;
    let mut seen = |m: &std::fs::Metadata| {
        let c = SystemTime::UNIX_EPOCH
            + Duration::new(m.ctime().max(0) as u64, m.ctime_nsec().max(0) as u32);
        newest = Some(newest.map_or(c, |n| n.max(c)));
    };
    for f in &files {
        f.hash(&mut h);
        match std::fs::metadata(f) {
            Ok(m) => {
                (m.dev(), m.ino(), m.size(), m.mtime(), m.mtime_nsec()).hash(&mut h);
                (m.ctime(), m.ctime_nsec()).hash(&mut h);
                seen(&m);
            }
            Err(_) => 0u8.hash(&mut h),
        }
    }
    for d in &dirs {
        d.hash(&mut h);
        let mut names: Vec<(OsString, bool)> = match std::fs::read_dir(d) {
            Ok(rd) => rd
                .flatten()
                .map(|e| (e.file_name(), e.file_type().is_ok_and(|t| t.is_dir())))
                .collect(),
            Err(_) => Vec::new(),
        };
        names.sort();
        names.hash(&mut h);
        if let Ok(m) = std::fs::metadata(d) {
            seen(&m);
        }
    }
    (Hasher::finish(&h), newest)
}

/// In a server (`JUSTRUST_CARGO_SERVE`), called right after `create_bcx`:
/// park and fork per request. Returns only in a forked child, which then
/// runs the rest of the build for its client. Not a server: returns.
pub fn park(bcx: &BuildContext<'_, '_>, has_logger: bool) {
    let Some((sock, started)) = SERVING.get() else {
        return;
    };
    // Every exit path below ends this start attempt.
    let _ = std::fs::remove_file(starting_marker(sock));
    if PARKED.swap(true, Ordering::SeqCst) {
        std::process::exit(0);
    }
    // A server never builds: anything that is not a clean park exits.
    let static_reason = if has_logger {
        Some("build logger")
    } else if bcx.build_config.unit_graph || bcx.build_config.dry_run {
        Some("unit graph or dry run")
    } else if !only_idle_threads() {
        Some("more than one thread")
    } else if bcx.gctx.jr_package_cache_locked() {
        Some("package cache lock held")
    } else {
        None
    };
    if let Some(why) = static_reason {
        let threads: Vec<String> = std::fs::read_dir("/proc/self/task")
            .map(|d| {
                d.flatten()
                    .filter_map(|e| std::fs::read_to_string(e.path().join("comm")).ok())
                    .map(|s| s.trim().to_owned())
                    .collect()
            })
            .unwrap_or_default();
        eprintln!("jr-cargo daemon: not parking: {why} (threads: {threads:?})");
        let _ = std::fs::write(nopark_marker(sock), why);
        std::process::exit(0);
    }
    let (stamp0, newest) = stamp(bcx);
    // A file changed while this server was loading: its state may predate
    // the change while the stamp already includes it.
    if newest.is_some_and(|n| n + Duration::from_millis(50) >= *started) {
        eprintln!("jr-cargo daemon: not parking: inputs changed during startup");
        std::process::exit(0);
    }
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    let pre_out = std::fs::read("/proc/self/fd/1").unwrap_or_default();
    let pre_err = std::fs::read("/proc/self/fd/2").unwrap_or_default();
    let listener = match bind(sock) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("jr-cargo daemon: bind {}: {e}", sock.display());
            std::process::exit(0);
        }
    };
    unsafe { libc::signal(libc::SIGCHLD, libc::SIG_IGN) };
    let _ = std::fs::write(sock.with_extension("pid"), std::process::id().to_string());
    sweep(sock);
    eprintln!("jr-cargo daemon: parked at {}", sock.display());
    loop {
        let mut pfd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut pfd, 1, TICK.as_millis() as i32) };
        if n == 0 {
            if let Some(why) = evict_reason(sock) {
                shutdown(sock, why);
            }
            continue;
        }
        touch(sock);
        let Ok((mut conn, _)) = listener.accept() else {
            continue;
        };
        let Ok((env, fds)) = recv_request(&conn) else {
            continue;
        };
        if fds.len() != 3 {
            continue;
        }
        if stamp(bcx).0 != stamp0 {
            let _ = conn.write_all(b"S");
            shutdown(sock, "stale");
        }
        match unsafe { libc::fork() } {
            0 => {
                drop(listener);
                become_child(conn, fds, env, &pre_out, &pre_err);
                return;
            }
            -1 => {
                let _ = conn.write_all(b"S");
            }
            _ => {}
        }
    }
}

/// Forking copies only the calling thread. Allowed besides it: the
/// `futures-timer` helper (started by the package downloader's progress
/// tick). It holds no cargo state; a child that needed a timer would wait
/// forever, but every package is downloaded before `create_bcx` returns.
fn only_idle_threads() -> bool {
    let Ok(d) = std::fs::read_dir("/proc/self/task") else {
        return false;
    };
    let me = unsafe { libc::gettid() }.to_string();
    d.flatten().all(|e| {
        e.file_name().to_str() == Some(me.as_str())
            || std::fs::read_to_string(e.path().join("comm"))
                .is_ok_and(|c| c.trim() == "futures-timer")
    })
}

fn touch(sock: &Path) {
    // A socket cannot be opened, so set its mtime by path.
    if let Ok(c) = std::ffi::CString::new(sock.as_os_str().as_encoded_bytes()) {
        unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), std::ptr::null(), 0) };
    }
}

fn mem_available_kb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let l = s.lines().find(|l| l.starts_with("MemAvailable:"))?;
    l.split_whitespace().nth(1)?.parse().ok()
}

/// Why this parked server should exit now, if at all.
fn evict_reason(sock: &Path) -> Option<&'static str> {
    // Unreachable without its socket (removed by hand, or a wiped runtime
    // directory): nothing could ever evict this server otherwise.
    let Ok(meta) = std::fs::symlink_metadata(sock) else {
        return Some("socket removed");
    };
    let mine = meta.modified().ok()?;
    if mine.elapsed().unwrap_or_default() >= IDLE {
        return Some("idle");
    }
    if mem_available_kb().is_some_and(|kb| kb < MEM_FLOOR_KB) {
        return Some("low memory");
    }
    let newer = std::fs::read_dir(sock.parent()?)
        .ok()?
        .flatten()
        .filter(|e| e.path() != sock && e.path().extension().is_some_and(|x| x == "sock"))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .filter(|m| *m > mine)
        .count();
    (newer >= MAX_PARKED).then_some("lru cap")
}

fn shutdown(sock: &Path, why: &str) -> ! {
    eprintln!("jr-cargo daemon: exiting ({why})");
    let _ = std::fs::remove_file(sock);
    for ext in ["pid", "out", "err"] {
        let _ = std::fs::remove_file(sock.with_extension(ext));
    }
    sweep(sock);
    std::process::exit(0)
}

/// Whether the server that wrote `<stem>.pid` is still running. `None`: no
/// readable pid file.
fn owner_alive(any_file_of_key: &Path) -> Option<bool> {
    let pid: i32 = std::fs::read_to_string(any_file_of_key.with_extension("pid"))
        .ok()?
        .trim()
        .parse()
        .ok()?;
    if pid <= 0 {
        return Some(false);
    }
    let r = unsafe { libc::kill(pid, 0) };
    Some(r == 0 || io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

/// Whether [`sweep`] may delete a file with extension `ext` that was last
/// modified `age` ago and whose server is `owner` (see [`owner_alive`]).
fn sweepable(ext: &str, age: Duration, owner: Option<bool>) -> bool {
    if owner == Some(true) || age < SWEEP_GRACE {
        return false;
    }
    match ext {
        // A "this invocation cannot park" memo outlives its server on purpose.
        "nopark" => age >= SWEEP_DAY,
        "sock" if owner.is_none() => age >= SWEEP_UNOWNED_SOCK,
        "sock" | "out" | "err" | "pid" | "starting" => true,
        _ => false,
    }
}

/// Removes what dead servers left in the runtime directory: sockets of
/// killed servers and the logs of every server that exited without parking.
/// Runs when a server parks and when it exits, never in a client.
fn sweep(own: &Path) {
    let Some(dir) = own.parent() else {
        return;
    };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.file_stem() == own.file_stem() {
            continue;
        }
        let Some(ext) = p.extension().and_then(|x| x.to_str()) else {
            continue;
        };
        // `DirEntry::metadata` does not follow symlinks.
        let Some(age) = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
        else {
            continue;
        };
        if age < SWEEP_GRACE {
            continue;
        }
        if sweepable(ext, age, owner_alive(&p)) {
            let _ = std::fs::remove_file(&p);
        }
    }
}

fn bind(sock: &Path) -> io::Result<UnixListener> {
    if UnixStream::connect(sock).is_ok() {
        return Err(io::Error::new(io::ErrorKind::AddrInUse, "server alive"));
    }
    let _ = std::fs::remove_file(sock);
    UnixListener::bind(sock)
}

fn become_child(
    conn: UnixStream,
    fds: Vec<OwnedFd>,
    env: Vec<(OsString, OsString)>,
    pre_out: &[u8],
    pre_err: &[u8],
) {
    let _ = CHILD_START.set(Instant::now());
    unsafe {
        libc::signal(libc::SIGCHLD, libc::SIG_DFL);
        libc::setpgid(0, 0);
    }
    let _ = (&conn).write_all(b"R");
    let write_all = |fd: RawFd, mut b: &[u8]| {
        while !b.is_empty() {
            let n = unsafe { libc::write(fd, b.as_ptr().cast(), b.len()) };
            if n <= 0 {
                break;
            }
            b = &b[n as usize..];
        }
    };
    write_all(fds[1].as_raw_fd(), pre_out);
    write_all(fds[2].as_raw_fd(), pre_err);
    for (i, fd) in fds.iter().enumerate() {
        unsafe { libc::dup2(fd.as_raw_fd(), i as RawFd) };
    }
    drop(fds);
    unsafe {
        for (k, _) in std::env::vars_os() {
            std::env::remove_var(k);
        }
        for (k, v) in &env {
            std::env::set_var(k, v);
        }
    }
    let _ = CHILD_ENV.set(env.into_iter().collect());
    // justrust samples the process tree of the cargo it started; this build
    // runs elsewhere, so name it (`justrust` record.rs follows this file).
    if let Some(dir) = std::env::var_os("JUSTRUST_RUN_DIR") {
        let _ = std::fs::write(
            Path::new(&dir).join("cargo-daemon.pid"),
            std::process::id().to_string(),
        );
    }
    if std::env::var_os("JUSTRUST_CARGO_DAEMON_TRACE").is_some() {
        eprintln!("jr-cargo daemon: served by a parked server");
    }
    let conn_fd = conn.as_raw_fd();
    // Client gone (killed, timed out): stop this build and its rustcs.
    if let Ok(watch) = conn.try_clone() {
        std::thread::spawn(move || {
            let mut b = [0u8; 1];
            let _ = (&watch).read(&mut b);
            if !FINISHED.load(Ordering::SeqCst) {
                unsafe { libc::kill(-libc::getpid(), libc::SIGKILL) };
            }
        });
    }
    std::mem::forget(conn);
    CHILD_CONN.store(conn_fd, Ordering::Relaxed);
}

fn send_with_fds(s: &UnixStream, data: &[u8], fds: &[RawFd]) -> io::Result<()> {
    let fd_bytes = std::mem::size_of_val(fds);
    unsafe {
        let space = libc::CMSG_SPACE(fd_bytes as u32) as usize;
        let mut cbuf = vec![0u64; space.div_ceil(8)];
        let mut iov = libc::iovec {
            iov_base: data.as_ptr() as *mut _,
            iov_len: data.len(),
        };
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let c = libc::CMSG_FIRSTHDR(&msg);
        (*c).cmsg_level = libc::SOL_SOCKET;
        (*c).cmsg_type = libc::SCM_RIGHTS;
        (*c).cmsg_len = libc::CMSG_LEN(fd_bytes as u32) as _;
        std::ptr::copy_nonoverlapping(fds.as_ptr(), libc::CMSG_DATA(c).cast(), fds.len());
        let n = libc::sendmsg(s.as_raw_fd(), &msg, libc::MSG_NOSIGNAL);
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut w = s;
        w.write_all(&data[n as usize..])
    }
}

fn recv_request(s: &UnixStream) -> io::Result<(Vec<(OsString, OsString)>, Vec<OwnedFd>)> {
    s.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut head = [0u8; 4];
    let mut fds = Vec::new();
    let n = unsafe {
        let space = libc::CMSG_SPACE(16 * 4) as usize;
        let mut cbuf = vec![0u64; space.div_ceil(8)];
        let mut iov = libc::iovec {
            iov_base: head.as_mut_ptr().cast(),
            iov_len: 4,
        };
        let mut msg: libc::msghdr = std::mem::zeroed();
        msg.msg_iov = &mut iov;
        msg.msg_iovlen = 1;
        msg.msg_control = cbuf.as_mut_ptr().cast();
        msg.msg_controllen = space as _;
        let n = libc::recvmsg(s.as_raw_fd(), &mut msg, libc::MSG_CMSG_CLOEXEC);
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut c = libc::CMSG_FIRSTHDR(&msg);
        while !c.is_null() {
            if (*c).cmsg_level == libc::SOL_SOCKET && (*c).cmsg_type == libc::SCM_RIGHTS {
                let len = (*c).cmsg_len as usize - libc::CMSG_LEN(0) as usize;
                let p: *const RawFd = libc::CMSG_DATA(c).cast();
                for i in 0..len / 4 {
                    fds.push(OwnedFd::from_raw_fd(*p.add(i)));
                }
            }
            c = libc::CMSG_NXTHDR(&msg, c);
        }
        n as usize
    };
    let mut r = s;
    if n < 4 {
        r.read_exact(&mut head[n..])?;
    }
    let len = u32::from_le_bytes(head) as usize;
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    let env = payload
        .split(|b| *b == 0)
        .filter(|e| !e.is_empty())
        .filter_map(|e| {
            let i = e.iter().position(|b| *b == b'=')?;
            Some((
                OsString::from_vec(e[..i].to_vec()),
                OsString::from_vec(e[i + 1..].to_vec()),
            ))
        })
        .collect();
    s.set_read_timeout(None)?;
    Ok((env, fds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subcommand_skips_flag_values() {
        let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            subcommand(&a(&[
                "cargo", "--config", "x=1", "-Zfoo", "check", "-p", "a"
            ])),
            Some("check")
        );
        assert_eq!(subcommand(&a(&["cargo", "+nightly", "test"])), Some("test"));
        assert_eq!(
            subcommand(&a(&["cargo", "-Z", "x", "build"])),
            Some("build")
        );
        assert_eq!(subcommand(&a(&["cargo", "--version"])), None);
    }

    #[test]
    fn flock_holders_reads_proc_locks() {
        let locks = "1: FLOCK  ADVISORY  WRITE 4242 fd:01:777 0 EOF\n\
                     1: -> FLOCK  ADVISORY  WRITE 4300 fd:01:777 0 EOF\n\
                     2: POSIX  ADVISORY  WRITE 4400 fd:01:777 0 EOF\n\
                     3: FLOCK  ADVISORY  READ 4500 fd:01:778 0 EOF\n\
                     4: FLOCK  ADVISORY  READ 4600 fd:01:777 0 EOF\n";
        assert_eq!(flock_holders(locks, 777), vec![4242, 4600]);
        assert_eq!(flock_holders(locks, 778), vec![4500]);
        assert!(flock_holders(locks, 1).is_empty());
    }

    /// A process that holds the lock and carries the server variable is
    /// stopped and its start marker removed. One without the variable (any
    /// other cargo) is left alone.
    #[test]
    fn stops_only_servers_holding_the_lock() {
        use std::process::{Command, Stdio};
        let dir = tempfile::tempdir().unwrap();
        let lock = dir.path().join(".package-cache");
        let sock = dir.path().join("k.sock");
        std::fs::write(starting_marker(&sock), b"").unwrap();
        let hold = |server: bool| {
            let mut cmd = Command::new("flock");
            cmd.arg("-o").arg(&lock).args(["sleep", "20"]);
            cmd.stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .env_remove(SERVE_ENV);
            if server {
                cmd.env(SERVE_ENV, &sock);
            }
            cmd.spawn().ok()
        };
        let held = |pid: u32| {
            let ino = std::fs::metadata(&lock).map(|m| m.ino()).unwrap_or(0);
            let locks = std::fs::read_to_string("/proc/locks").unwrap_or_default();
            flock_holders(&locks, ino).contains(&(pid as i32))
        };
        let wait_held = |pid: u32| {
            let end = Instant::now() + Duration::from_secs(10);
            while !held(pid) && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(5));
            }
            held(pid)
        };
        // No flock(1) on this machine: nothing to test with.
        let Some(mut other) = hold(false) else {
            return;
        };
        assert!(wait_held(other.id()));
        assert!(!stop_starting_servers_holding(&lock));
        assert!(held(other.id()));
        other.kill().unwrap();
        other.wait().unwrap();

        let mut server = hold(true).unwrap();
        assert!(wait_held(server.id()));
        assert!(stop_starting_servers_holding(&lock));
        server.wait().unwrap();
        assert!(!held(server.id()));
        assert!(!starting_marker(&sock).exists());
    }

    #[test]
    fn volatile_vars() {
        assert!(volatile("JUSTRUST_RUN_DIR"));
        assert!(volatile("KITTY_WINDOW_ID"));
        assert!(!volatile("RUSTFLAGS"));
        assert!(!volatile("CARGO_TARGET_DIR"));
    }

    #[test]
    fn sweep_rules() {
        let s = Duration::from_secs;
        // A live server keeps everything, however old.
        assert!(!sweepable("sock", SWEEP_DAY * 3, Some(true)));
        assert!(!sweepable("err", SWEEP_DAY * 3, Some(true)));
        // A starting server has fresh logs and no pid file yet.
        assert!(!sweepable("out", s(5), None));
        assert!(!sweepable("starting", s(5), None));
        // Dead owner: logs, pid file and socket go after the grace period.
        assert!(sweepable("sock", s(300), Some(false)));
        assert!(sweepable("pid", s(300), Some(false)));
        assert!(sweepable("err", s(300), None));
        // A socket without a pid file may belong to an older live server.
        assert!(!sweepable("sock", s(300), None));
        assert!(sweepable("sock", SWEEP_UNOWNED_SOCK, None));
        assert!(!sweepable("nopark", s(300), Some(false)));
        assert!(sweepable("nopark", SWEEP_DAY, None));
        assert!(!sweepable("toml", SWEEP_DAY, None));
    }

    #[test]
    fn sweep_removes_only_dead_servers_files() {
        let dir = std::env::temp_dir().join(format!("jr-daemon-sweep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let old = SystemTime::now() - Duration::from_secs(600);
        let mk = |name: &str, body: &str, old_file: bool| {
            let p = dir.join(name);
            std::fs::write(&p, body).unwrap();
            if old_file {
                std::fs::File::options()
                    .write(true)
                    .open(&p)
                    .unwrap()
                    .set_modified(old)
                    .unwrap();
            }
            p
        };
        let me = std::process::id().to_string();
        // Live server (this process), old files: kept.
        let live = [mk("aa.pid", &me, true), mk("aa.err", "x", true)];
        // Dead server (no such pid), old files: removed.
        let dead = [
            mk("bb.pid", "0", true),
            mk("bb.err", "x", true),
            mk("bb.out", "", true),
        ];
        // No pid file: old logs removed, fresh ones (a server starting) kept.
        let orphan_log = mk("cc.err", "x", true);
        let starting = [mk("dd.err", "", false), mk("dd.starting", "", false)];
        // The sweeping server's own files are never touched.
        let own = mk("ee.err", "x", true);
        sweep(&dir.join("ee.sock"));
        for p in live.iter().chain(&starting).chain([&own]) {
            assert!(p.exists(), "{} should be kept", p.display());
        }
        for p in dead.iter().chain([&orphan_log]) {
            assert!(!p.exists(), "{} should be removed", p.display());
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
