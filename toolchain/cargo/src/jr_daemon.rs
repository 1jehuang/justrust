//! justrust: jr-cargo fork server (opt-in, `JUSTRUST_CARGO_DAEMON=1`).
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
//! A server exits after 15 minutes without requests.

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

/// The socket for this invocation, if it may use the daemon.
fn eligible_socket() -> Option<PathBuf> {
    if std::env::var_os(ENABLE_ENV).is_none_or(|v| v != "1")
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
    eprintln!("jr-cargo daemon: parked at {}", sock.display());
    loop {
        let mut pfd = libc::pollfd {
            fd: listener.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let n = unsafe { libc::poll(&mut pfd, 1, IDLE.as_millis() as i32) };
        if n == 0 {
            shutdown(sock, "idle");
        }
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

fn shutdown(sock: &Path, why: &str) -> ! {
    eprintln!("jr-cargo daemon: exiting ({why})");
    let _ = std::fs::remove_file(sock);
    std::process::exit(0)
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
    fn volatile_vars() {
        assert!(volatile("JUSTRUST_RUN_DIR"));
        assert!(volatile("KITTY_WINDOW_ID"));
        assert!(!volatile("RUSTFLAGS"));
        assert!(!volatile("CARGO_TARGET_DIR"));
    }
}
