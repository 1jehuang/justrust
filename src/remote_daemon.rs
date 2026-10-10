//! The local sync daemon: `justrust __daemon`.
//!
//! One per user, started on demand by the first remote build and kept alive
//! while builds keep coming (exits after an hour without clients, or when
//! the backend changes). It owns:
//!
//! - one ssh session to `justrust __agent` on the machine,
//! - the set of source roots clients have asked for, each with the manifest
//!   of what the machine has,
//! - an inotify watch on those roots, so edits are pushed as they are saved,
//!   before any build asks.
//!
//! Clients connect to `~/.justrust/remote/daemon.sock` and send `Run` (or
//! `Ping`/`Shutdown`). For a run the daemon rescans the requested roots,
//! pushes whatever is still out of date, waits for the agent's barrier ack,
//! answers `Flushed`, then relays the run's `Out` frames and `Exit`.
//!
//! Failure policy: any infrastructure problem becomes an `Error` frame and
//! the client falls back to building locally. The daemon never guesses that
//! the machine is in sync: after a reconnect every root is rescanned against
//! a fresh rsync.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, BufWriter};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::remote_backend::{self, Backend, Target};
use crate::remote_proto::{Msg, VERSION, read_frame, write_frame};
use crate::remote_sync::{self, Manifest};

const IDLE_EXIT: Duration = Duration::from_secs(3600);

pub fn sock_path() -> Result<PathBuf> {
    Ok(crate::remote::dir()?.join("daemon.sock"))
}

pub fn build_id() -> String {
    std::env::current_exe()
        .and_then(std::fs::metadata)
        .map(|m| {
            use std::os::unix::fs::MetadataExt;
            format!("{}-{}", m.len(), m.mtime())
        })
        .unwrap_or_default()
}

// ------------------------------------------------------------------ client

/// Connect to the daemon, starting it if needed (or replacing a daemon from
/// an older justrust binary).
pub fn connect() -> Result<UnixStream> {
    let sock = sock_path()?;
    if let Ok(s) = UnixStream::connect(&sock) {
        match ping(&s) {
            Ok(build) if build == build_id() => return Ok(s),
            _ => {
                let _ = shutdown_via(&s);
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    spawn()?;
    let t = Instant::now();
    loop {
        if let Ok(s) = UnixStream::connect(&sock) {
            return Ok(s);
        }
        if t.elapsed() > Duration::from_secs(5) {
            bail!("sync daemon did not start (see ~/.justrust/remote/daemon.log)");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn ping(s: &UnixStream) -> Result<String> {
    s.set_read_timeout(Some(Duration::from_secs(2)))?;
    let mut w = s.try_clone()?;
    write_frame(&mut w, &Msg::Ping, &[])?;
    let mut r = s.try_clone()?;
    let res = match read_frame(&mut r)? {
        Some((Msg::Pong { build, .. }, _)) => Ok(build),
        _ => bail!("bad pong"),
    };
    s.set_read_timeout(None)?;
    res
}

fn shutdown_via(s: &UnixStream) -> Result<()> {
    let mut w = s.try_clone()?;
    write_frame(&mut w, &Msg::Shutdown, &[])
}

/// Stop a running daemon (backend changed or `remote use off`).
pub fn stop() {
    if let Ok(p) = sock_path()
        && let Ok(s) = UnixStream::connect(&p)
    {
        let _ = shutdown_via(&s);
    }
}

#[allow(dead_code)] // used by `remote status` (in progress)
pub fn status() -> Option<(Vec<String>, String, u64, bool)> {
    let s = UnixStream::connect(sock_path().ok()?).ok()?;
    s.set_read_timeout(Some(Duration::from_millis(500))).ok()?;
    let mut w = s.try_clone().ok()?;
    write_frame(&mut w, &Msg::Ping, &[]).ok()?;
    let mut r = s;
    match read_frame(&mut r).ok()?? {
        (
            Msg::Pong {
                roots,
                machine,
                pushed,
                connected,
                ..
            },
            _,
        ) => Some((roots, machine, pushed, connected)),
        _ => None,
    }
}

fn spawn() -> Result<()> {
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(crate::remote::dir()?.join("daemon.log"))?;
    let mut c = Command::new(std::env::current_exe()?);
    c.arg("__daemon")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    use std::os::unix::process::CommandExt;
    // Own session: survives the agent's terminal and its Ctrl+C.
    c.process_group(0);
    c.spawn().context("starting the sync daemon")?;
    Ok(())
}

// ------------------------------------------------------------------ daemon

struct Root {
    manifest: Manifest,
}

/// Messages from the agent, routed to whoever waits for them.
#[derive(Default)]
struct Routes {
    barriers: HashMap<u64, Sender<()>>,
    runs: HashMap<u64, Sender<(Msg, Vec<u8>)>>,
}

struct Conn {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    routes: Arc<Mutex<Routes>>,
    alive: Arc<(Mutex<bool>, Condvar)>,
    target: Target,
    prefix: String,
}

impl Drop for Conn {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct State {
    backend: Backend,
    conn: Option<Conn>,
    roots: BTreeMap<PathBuf, Root>,
    next_id: u64,
    pushed: u64,
    last_client: Instant,
}

type Shared = Arc<Mutex<State>>;

pub fn main() -> ! {
    let code = match daemon() {
        Ok(()) => 0,
        Err(e) => {
            eprintln!(
                "[{}] justrust daemon: {e:#}",
                chrono::Local::now().format("%F %T")
            );
            1
        }
    };
    std::process::exit(code)
}

fn log(msg: &str) {
    eprintln!("[{}] {msg}", chrono::Local::now().format("%F %T"));
}

fn daemon() -> Result<()> {
    let backend = remote_backend::load().context("no remote backend configured")?;
    let sock = sock_path()?;
    // Exclusive: a second daemon exits instead of stealing the socket.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(crate::remote::dir()?.join("daemon.lock"))?;
    use std::os::fd::AsRawFd;
    // SAFETY: flock on a file descriptor we own.
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        bail!("another daemon is running");
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    log(&format!("daemon up, backend {}", backend.label()));
    let state: Shared = Arc::new(Mutex::new(State {
        backend,
        conn: None,
        roots: BTreeMap::new(),
        next_id: 1,
        pushed: 0,
        last_client: Instant::now(),
    }));

    // Watcher: pushes edits as they land, debounced.
    let (wtx, wrx) = channel::<PathBuf>();
    let watcher = crate::remote_watch::Watcher::new(wtx)?;
    let watcher = Arc::new(Mutex::new(watcher));
    {
        let state = state.clone();
        std::thread::spawn(move || push_loop(state, wrx));
    }
    {
        let state = state.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                let idle = state
                    .lock()
                    .map(|s| s.last_client.elapsed() > IDLE_EXIT)
                    .unwrap_or(true);
                if idle {
                    log("idle, exiting");
                    std::process::exit(0);
                }
            }
        });
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        let watcher = watcher.clone();
        std::thread::spawn(move || {
            if let Err(e) = client(stream, &state, &watcher) {
                log(&format!("client: {e:#}"));
            }
        });
    }
    Ok(())
}

/// Debounce watch events, then push the touched roots.
fn push_loop(state: Shared, rx: Receiver<PathBuf>) {
    while let Ok(first) = rx.recv() {
        let mut touched = vec![first];
        // Editors write a burst (temp file, rename, chmod): settle briefly.
        while let Ok(p) = rx.recv_timeout(Duration::from_millis(15)) {
            touched.push(p);
        }
        let Ok(mut st) = state.lock() else { return };
        if st.conn.is_none() {
            continue;
        }
        let roots: Vec<PathBuf> = st
            .roots
            .keys()
            .filter(|r| touched.iter().any(|t| t.starts_with(r)))
            .cloned()
            .collect();
        for r in roots {
            if let Err(e) = push_root(&mut st, &r) {
                log(&format!("push {}: {e:#}", r.display()));
                st.conn = None;
                break;
            }
        }
    }
}

fn client(
    stream: UnixStream,
    state: &Shared,
    watcher: &Arc<Mutex<crate::remote_watch::Watcher>>,
) -> Result<()> {
    let mut r = BufReader::new(stream.try_clone()?);
    let mut w = BufWriter::new(stream.try_clone()?);
    while let Some((msg, _)) = read_frame(&mut r)? {
        if let Ok(mut s) = state.lock() {
            s.last_client = Instant::now();
        }
        match msg {
            Msg::Ping => {
                let s = state.lock().unwrap();
                write_frame(
                    &mut w,
                    &Msg::Pong {
                        roots: s.roots.keys().map(|r| r.display().to_string()).collect(),
                        machine: s.backend.label(),
                        pushed: s.pushed,
                        connected: s.conn.is_some(),
                        build: build_id(),
                    },
                    &[],
                )?;
            }
            Msg::Shutdown => {
                log("shutdown requested");
                std::process::exit(0);
            }
            Msg::Run {
                cwd,
                args,
                env,
                roots,
                start,
                run_id,
                ..
            } => {
                let res = run(
                    state, watcher, &mut r, &mut w, cwd, args, env, roots, start, run_id,
                );
                if let Err(e) = res {
                    let _ = write_frame(
                        &mut w,
                        &Msg::Error {
                            id: 0,
                            msg: format!("{e:#}"),
                        },
                        &[],
                    );
                }
                return Ok(());
            }
            _ => {}
        }
    }
    Ok(())
}

/// Connect to the agent (starting the machine only when `start`), mirror
/// every known root with rsync, and begin reading agent frames.
fn ensure_conn(st: &mut State, start: bool) -> Result<()> {
    if let Some(c) = &st.conn
        && *c.alive.0.lock().unwrap()
    {
        return Ok(());
    }
    st.conn = None;
    if !start && !st.backend.probably_up() {
        bail!("remote machine is not running");
    }
    let target = st.backend.ensure_ready()?;
    crate::remote_build::ensure_remote_justrust(&target)?;
    let mut child = target
        .command()
        .arg("exec ~/.cargo/bin/justrust __agent")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting the remote agent")?;
    let mut stdin = BufWriter::new(child.stdin.take().unwrap());
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    write_frame(&mut stdin, &Msg::Hello { version: VERSION }, &[])?;
    match read_frame(&mut stdout)? {
        Some((Msg::Hello { version }, _)) if version == VERSION => {}
        other => {
            let mut err = String::new();
            if let Some(mut e) = child.stderr.take() {
                use std::io::Read;
                let _ = e.read_to_string(&mut err);
            }
            bail!("remote agent handshake failed: {other:?} {}", err.trim());
        }
    }
    let routes: Arc<Mutex<Routes>> = Arc::default();
    let alive = Arc::new((Mutex::new(true), Condvar::new()));
    {
        let routes = routes.clone();
        let alive = alive.clone();
        std::thread::spawn(move || {
            while let Ok(Some((msg, payload))) = read_frame(&mut stdout) {
                let mut r = routes.lock().unwrap();
                match &msg {
                    Msg::BarrierAck { id } => {
                        if let Some(tx) = r.barriers.remove(id) {
                            let _ = tx.send(());
                        }
                    }
                    Msg::Out { id, .. } | Msg::Exit { id, .. } | Msg::Error { id, .. }
                        if *id != 0 =>
                    {
                        if let Some(tx) = r.runs.get(id) {
                            let _ = tx.send((msg.clone(), payload));
                        }
                        if matches!(msg, Msg::Exit { .. } | Msg::Error { .. }) {
                            r.runs.remove(id);
                        }
                    }
                    Msg::Error { msg, .. } => log(&format!("agent: {msg}")),
                    _ => {}
                }
            }
            // Connection lost: fail every waiter so clients fall back.
            *alive.0.lock().unwrap() = false;
            alive.1.notify_all();
            let mut r = routes.lock().unwrap();
            r.barriers.clear();
            for (id, tx) in r.runs.drain() {
                let _ = tx.send((
                    Msg::Error {
                        id,
                        msg: "connection to the remote machine was lost".into(),
                    },
                    Vec::new(),
                ));
            }
        });
    }
    log(&format!("connected to {}", target.dest));
    st.conn = Some(Conn {
        child,
        stdin,
        routes,
        alive,
        target,
        prefix: String::new(),
    });
    // Everything the machine had may be stale: start from a full mirror.
    let roots: Vec<PathBuf> = st.roots.keys().cloned().collect();
    for r in roots {
        st.roots.remove(&r);
        add_root(st, &r)?;
    }
    Ok(())
}

fn remote_path(prefix: &str, p: &Path) -> String {
    if prefix.is_empty() {
        p.display().to_string()
    } else {
        format!("{prefix}{}", p.display())
    }
}

/// Start tracking a root: rsync it once, then remember its manifest.
fn add_root(st: &mut State, root: &Path) -> Result<()> {
    let conn = st.conn.as_ref().context("not connected")?;
    let remote = remote_path(&conn.prefix, root);
    if !remote_sync::prepare_dir(&conn.target, &remote)? {
        bail!("cannot create {remote} on the remote machine");
    }
    // Scan before rsync: a file edited during the copy then differs from
    // the manifest and is pushed again by the next flush.
    let manifest = remote_sync::scan(root)?;
    let t = Instant::now();
    let n = remote_sync::rsync(&conn.target, root, &remote)?;
    log(&format!(
        "mirrored {} ({} files sent) in {:.2}s",
        root.display(),
        n,
        t.elapsed().as_secs_f64()
    ));
    st.roots.insert(root.to_path_buf(), Root { manifest });
    Ok(())
}

/// Push every difference between `root` and its manifest. Returns files sent.
fn push_root(st: &mut State, root: &Path) -> Result<usize> {
    let new = remote_sync::scan(root)?;
    let d = {
        let r = st.roots.get(root).context("unknown root")?;
        remote_sync::diff(&r.manifest, &new)
    };
    if d.is_empty() {
        return Ok(0);
    }
    let conn = st.conn.as_mut().context("not connected")?;
    if d.changed.len() > remote_sync::RSYNC_ABOVE {
        // A branch switch or a big generated change: one rsync is cheaper.
        let remote = remote_path(&conn.prefix, root);
        remote_sync::rsync(&conn.target, root, &remote)?;
    } else {
        for rel in &d.changed {
            let p = root.join(rel);
            let Ok(data) = std::fs::read(&p) else {
                continue; // gone between scan and read: the next scan deletes it
            };
            let mode = new.get(rel).map_or(0o644, |s| s.mode);
            write_frame(
                &mut conn.stdin,
                &Msg::Write {
                    path: remote_path("", &p),
                    mode,
                },
                &data,
            )?;
        }
        for rel in &d.deleted {
            write_frame(
                &mut conn.stdin,
                &Msg::Delete {
                    path: remote_path("", &root.join(rel)),
                },
                &[],
            )?;
        }
    }
    let n = d.changed.len() + d.deleted.len();
    st.pushed += n as u64;
    if let Some(r) = st.roots.get_mut(root) {
        r.manifest = new;
    }
    Ok(n)
}

/// Wait until the agent has applied everything sent so far.
fn barrier(st: &mut State) -> Result<()> {
    let id = st.next_id;
    st.next_id += 1;
    let conn = st.conn.as_mut().context("not connected")?;
    let (tx, rx) = channel();
    conn.routes.lock().unwrap().barriers.insert(id, tx);
    write_frame(&mut conn.stdin, &Msg::Barrier { id }, &[])?;
    rx.recv_timeout(Duration::from_secs(30))
        .map_err(|_| anyhow::anyhow!("remote machine did not acknowledge the sync"))
}

#[allow(clippy::too_many_arguments)]
fn run(
    state: &Shared,
    watcher: &Arc<Mutex<crate::remote_watch::Watcher>>,
    r: &mut BufReader<UnixStream>,
    w: &mut BufWriter<UnixStream>,
    cwd: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
    roots: Vec<String>,
    start: bool,
    run_id: String,
) -> Result<()> {
    let t = Instant::now();
    let (id, rx, machine, prefix, files) = {
        let mut st = state.lock().unwrap();
        ensure_conn(&mut st, start)?;
        let mut files = 0;
        for root in &roots {
            let root = PathBuf::from(root);
            if !st.roots.contains_key(&root) {
                add_root(&mut st, &root)?;
                let dirs = remote_sync::dirs(&root, &st.roots[&root].manifest);
                if let Ok(mut wt) = watcher.lock() {
                    wt.watch_all(&dirs);
                }
            } else {
                files += push_root(&mut st, &root)?;
            }
        }
        barrier(&mut st)?;
        let id = st.next_id;
        st.next_id += 1;
        let machine = st.backend.label();
        let conn = st.conn.as_mut().context("not connected")?;
        let (tx, rx) = channel();
        conn.routes.lock().unwrap().runs.insert(id, tx);
        write_frame(
            &mut conn.stdin,
            &Msg::Run {
                id,
                cwd: cwd.clone(),
                args,
                env,
                roots: Vec::new(),
                start: false,
                run_id,
            },
            &[],
        )?;
        (id, rx, machine, conn.prefix.clone(), files)
    };
    write_frame(
        w,
        &Msg::Flushed {
            id,
            files,
            ms: t.elapsed().as_secs_f64() * 1000.0,
            machine,
            prefix,
        },
        &[],
    )?;
    // A client that sends Cancel or goes away (Ctrl+C, killed agent) cancels
    // the remote run, unless it already finished.
    let finished = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let cancel_state = state.clone();
    let cr = r.get_ref().try_clone()?;
    {
        let finished = finished.clone();
        std::thread::spawn(move || {
            let mut buf = BufReader::new(cr);
            let _ = read_frame(&mut buf);
            if finished.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            if let Ok(mut st) = cancel_state.lock()
                && let Some(c) = st.conn.as_mut()
            {
                let _ = write_frame(&mut c.stdin, &Msg::Cancel { id }, &[]);
            }
        });
    }
    while let Ok((msg, payload)) = rx.recv() {
        let done = matches!(msg, Msg::Exit { .. } | Msg::Error { .. });
        if done {
            finished.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        if write_frame(w, &msg, &payload).is_err() || done {
            break;
        }
    }
    Ok(())
}
