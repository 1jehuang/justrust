//! The local sync daemon: `justrust __daemon`.
//!
//! One per user, started on demand by the first remote build and kept alive
//! while builds keep coming (exits after an hour without clients or runs, or
//! when the backend changes). It owns:
//!
//! - one ssh session to `justrust __agent` on the machine (a `Link`),
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
//! Locking: `Info` (what `Ping` reports) is only ever held for a moment.
//! `Sync` (the link, the roots, their manifests) is held across network
//! operations: connecting, rsync, pushes, barriers. A run holds it only
//! until its `Run` frame is sent, so concurrent runs overlap on the machine,
//! and `Ping`/status never waits for the network.
//!
//! Failure policy: any infrastructure problem becomes an `Error` frame and
//! the client falls back to building locally. The daemon never guesses that
//! the machine is in sync: after a reconnect every root is mirrored again
//! with rsync, and a write the agent could not apply fails the next barrier.

use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, BufWriter, Read};
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::remote_backend::{self, Backend};
use crate::remote_proto::{Msg, VERSION, read_frame, write_frame};
use crate::remote_sync::{self, MAX_FILE, Manifest, Mirror, UNKNOWN};
use crate::remote_watch::Watcher;

const IDLE_EXIT: Duration = Duration::from_secs(3600);
const BARRIER_TIMEOUT: Duration = Duration::from_secs(60);

pub fn sock_path() -> Result<PathBuf> {
    Ok(crate::remote::dir()?.join("daemon.sock"))
}

/// Identity of this process's binary, fixed at first use: after `cargo
/// install` replaces the file, a running daemon still reports the build it
/// started from, so clients of the new build replace it.
pub fn build_id() -> String {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        std::env::current_exe()
            .and_then(std::fs::metadata)
            .map(|m| format!("{}-{}", m.len(), m.mtime()))
            .unwrap_or_default()
    })
    .clone()
}

// ------------------------------------------------------------------ client

/// Connect to the daemon, starting it if needed (or replacing a daemon from
/// another justrust build).
pub fn connect() -> Result<UnixStream> {
    let sock = sock_path()?;
    let me = build_id();
    if let Ok(s) = UnixStream::connect(&sock) {
        match ping(&s) {
            // A client whose own binary was replaced (empty id) cannot tell
            // which build is current: use whatever daemon runs.
            Ok(p) if p.build == me || me.is_empty() => return Ok(s),
            _ => shutdown_and_wait(&s),
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

/// What a daemon reports about itself.
#[derive(Debug, Default)]
pub struct Pong {
    pub roots: Vec<String>,
    pub machine: String,
    pub pushed: u64,
    pub connected: bool,
    pub build: String,
    /// Runs in flight and files too large to mirror. Read by tests now,
    /// not yet shown by `remote status` (src/remote.rs).
    #[allow(dead_code)]
    pub active: u64,
    #[allow(dead_code)]
    pub skipped: u64,
}

fn ping_with(s: &UnixStream, timeout: Duration) -> Result<Pong> {
    s.set_read_timeout(Some(timeout))?;
    let mut w = s.try_clone()?;
    write_frame(&mut w, &Msg::Ping, &[])?;
    let mut r = s.try_clone()?;
    let res = match read_frame(&mut r)? {
        Some((
            Msg::Pong {
                roots,
                machine,
                pushed,
                connected,
                build,
                active,
                skipped,
            },
            _,
        )) => Ok(Pong {
            roots,
            machine,
            pushed,
            connected,
            build,
            active,
            skipped,
        }),
        _ => bail!("bad pong"),
    };
    s.set_read_timeout(None)?;
    res
}

fn ping(s: &UnixStream) -> Result<Pong> {
    ping_with(s, Duration::from_secs(2))
}

/// Ask a daemon to exit and wait until it has (its end of the socket
/// closes), so a replacement never races it for the lock or the socket.
fn shutdown_and_wait(s: &UnixStream) {
    let Ok(mut w) = s.try_clone() else { return };
    if write_frame(&mut w, &Msg::Shutdown, &[]).is_err() {
        return;
    }
    let _ = s.set_read_timeout(Some(Duration::from_secs(5)));
    let mut r = s;
    let mut buf = [0u8; 256];
    while matches!(r.read(&mut buf), Ok(n) if n > 0) {}
}

/// Stop a running daemon (backend changed or `remote use off`).
pub fn stop() {
    if let Ok(p) = sock_path()
        && let Ok(s) = UnixStream::connect(&p)
    {
        shutdown_and_wait(&s);
    }
}

/// `(roots, machine, files pushed, connected)` of the running daemon.
/// Never waits behind a sync: the daemon answers pings from a separate lock.
pub fn status() -> Option<(Vec<String>, String, u64, bool)> {
    let s = UnixStream::connect(sock_path().ok()?).ok()?;
    let p = ping_with(&s, Duration::from_millis(500)).ok()?;
    Some((p.roots, p.machine, p.pushed, p.connected))
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

/// Messages from the agent, routed to whoever waits for them.
#[derive(Default)]
struct Routes {
    /// Set once the agent's stdout closed: nothing registered after that
    /// would ever be answered.
    dead: bool,
    barriers: HashMap<u64, Sender<Vec<String>>>,
    runs: HashMap<u64, Sender<(Msg, Vec<u8>)>>,
}

/// One ssh session to the agent.
struct Link {
    child: Mutex<Child>,
    stdin: Mutex<BufWriter<ChildStdin>>,
    routes: Mutex<Routes>,
    mirror: Mirror,
}

impl Link {
    fn alive(&self) -> bool {
        !self.routes.lock().unwrap().dead
    }

    fn send(&self, m: &Msg, payload: &[u8]) -> Result<()> {
        write_frame(&mut *self.stdin.lock().unwrap(), m, payload)
    }

    fn kill(&self) {
        let mut c = self.child.lock().unwrap();
        let _ = c.kill();
        let _ = c.wait();
    }
}

struct Root {
    manifest: Manifest,
    /// Files over `MAX_FILE`, not mirrored.
    oversized: Vec<PathBuf>,
}

/// Everything network operations touch. Held across them.
struct Sync {
    backend: Backend,
    link: Option<Arc<Link>>,
    roots: BTreeMap<PathBuf, Root>,
}

/// What `Ping` reports. Only ever locked briefly.
#[derive(Default)]
struct Info {
    roots: Vec<String>,
    machine: String,
    pushed: u64,
    connected: bool,
    active: u64,
    skipped: u64,
    last_client: Option<Instant>,
}

struct Daemon {
    sync: Mutex<Sync>,
    info: Mutex<Info>,
    next_id: AtomicU64,
    watcher: Watcher,
    /// Device and inode of our socket, to remove it on exit only if it is
    /// still ours.
    sock: (PathBuf, u64, u64),
    /// The daemon lock. Released when draining so a replacement can start.
    lock: Mutex<Option<std::fs::File>>,
    /// Shut down but still relaying runs that were in flight.
    draining: AtomicBool,
}

impl Daemon {
    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn set_info(&self, f: impl FnOnce(&mut Info)) {
        if let Ok(mut i) = self.info.lock() {
            f(&mut i);
        }
    }

    /// Stop being the daemon: remove our socket (if a replacement has not
    /// already bound a new one) and release the lock.
    fn retire(&self) {
        let (p, dev, ino) = &self.sock;
        if std::fs::metadata(p).is_ok_and(|m| m.dev() == *dev && m.ino() == *ino) {
            let _ = std::fs::remove_file(p);
        }
        if let Ok(mut l) = self.lock.lock() {
            l.take();
        }
    }

    fn exit(&self, why: &str) -> ! {
        log(why);
        self.retire();
        std::process::exit(0)
    }

    /// `Shutdown` (a newer build or a backend change): retire at once so a
    /// replacement can start, but let runs in flight finish streaming
    /// rather than cutting off another agent's build.
    fn shutdown(self: &Arc<Self>) {
        let active = self.info.lock().map_or(0, |i| i.active);
        if active == 0 {
            self.exit("shutdown requested");
        }
        if self.draining.swap(true, Ordering::SeqCst) {
            return;
        }
        log(&format!(
            "shutdown requested, finishing {active} run(s) in flight"
        ));
        self.retire();
        let d = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(100));
                if d.info.lock().map_or(0, |i| i.active) == 0 {
                    d.exit("runs in flight finished, exiting");
                }
            }
        });
    }
}

pub fn main() -> ! {
    let code = match daemon() {
        Ok(()) => 0,
        Err(e) => {
            log(&format!("justrust daemon: {e:#}"));
            1
        }
    };
    std::process::exit(code)
}

fn log(msg: &str) {
    eprintln!(
        "[{}] [{}] {msg}",
        chrono::Local::now().format("%F %T"),
        std::process::id()
    );
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
    let md = std::fs::metadata(&sock)?;
    log(&format!(
        "daemon up, backend {}, build {}",
        backend.label(),
        build_id()
    ));
    let (wtx, wrx) = channel::<PathBuf>();
    let d = Arc::new(Daemon {
        info: Mutex::new(Info {
            machine: backend.label(),
            last_client: Some(Instant::now()),
            ..Default::default()
        }),
        sync: Mutex::new(Sync {
            backend,
            link: None,
            roots: BTreeMap::new(),
        }),
        next_id: AtomicU64::new(1),
        watcher: Watcher::new(wtx)?,
        sock: (sock.clone(), md.dev(), md.ino()),
        lock: Mutex::new(Some(lock)),
        draining: AtomicBool::new(false),
    });
    {
        let d = d.clone();
        std::thread::spawn(move || push_loop(&d, wrx));
    }
    {
        let d = d.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(60));
                let idle = d.info.lock().map_or(true, |i| {
                    i.active == 0 && i.last_client.is_none_or(|t| t.elapsed() > IDLE_EXIT)
                });
                if idle {
                    d.exit("idle, exiting");
                }
            }
        });
    }
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let d = d.clone();
        std::thread::spawn(move || {
            if let Err(e) = client(stream, &d) {
                log(&format!("client: {e:#}"));
            }
        });
    }
    Ok(())
}

/// Debounce watch events, then push the touched roots.
fn push_loop(d: &Daemon, rx: Receiver<PathBuf>) {
    while let Ok(first) = rx.recv() {
        let mut touched = vec![first];
        // Editors write a burst (temp file, rename, chmod): settle briefly.
        while let Ok(p) = rx.recv_timeout(Duration::from_millis(15)) {
            touched.push(p);
        }
        if d.draining.load(Ordering::SeqCst) {
            continue;
        }
        let Ok(mut st) = d.sync.lock() else { return };
        if !st.link.as_ref().is_some_and(|l| l.alive()) {
            continue;
        }
        let roots: Vec<PathBuf> = st
            .roots
            .keys()
            .filter(|r| touched.iter().any(|t| t.starts_with(r)))
            .cloned()
            .collect();
        for r in roots {
            if let Err(e) = push_root(d, &mut st, &r) {
                log(&format!("push {}: {e:#}", r.display()));
                drop_link(d, &mut st);
                break;
            }
        }
    }
}

fn client(stream: UnixStream, d: &Arc<Daemon>) -> Result<()> {
    let mut r = BufReader::new(stream.try_clone()?);
    let mut w = BufWriter::new(stream.try_clone()?);
    while let Some((msg, _)) = read_frame(&mut r)? {
        d.set_info(|i| i.last_client = Some(Instant::now()));
        match msg {
            Msg::Ping => {
                let pong = {
                    let i = d.info.lock().unwrap();
                    Msg::Pong {
                        roots: i.roots.clone(),
                        machine: i.machine.clone(),
                        pushed: i.pushed,
                        connected: i.connected,
                        build: build_id(),
                        active: i.active,
                        skipped: i.skipped,
                    }
                };
                write_frame(&mut w, &pong, &[])?;
            }
            Msg::Shutdown => {
                d.shutdown();
                return Ok(());
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
                d.set_info(|i| i.active += 1);
                let res = run(d, &mut r, &mut w, cwd, args, env, roots, start, run_id);
                d.set_info(|i| i.active = i.active.saturating_sub(1));
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

fn drop_link(d: &Daemon, st: &mut Sync) {
    if let Some(l) = st.link.take() {
        l.kill();
    }
    d.set_info(|i| i.connected = false);
}

/// Connect to the agent (starting the machine only when `start`), mirror
/// every known root with rsync, and begin reading agent frames.
fn ensure_link(d: &Daemon, st: &mut Sync, start: bool) -> Result<Arc<Link>> {
    if let Some(l) = &st.link {
        if l.alive() {
            return Ok(l.clone());
        }
        log("connection to the agent was lost, reconnecting");
    }
    drop_link(d, st);
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
    // The agent reports write failures on stderr: drain it into our log so
    // a full pipe never stalls it.
    if let Some(mut e) = child.stderr.take() {
        std::thread::spawn(move || {
            let mut s = String::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = e.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(i) = s.find('\n') {
                    log(&format!("agent: {}", &s[..i]));
                    s.drain(..=i);
                }
            }
        });
    }
    let mut stdin = BufWriter::new(child.stdin.take().unwrap());
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    write_frame(&mut stdin, &Msg::Hello { version: VERSION }, &[])?;
    match read_frame(&mut stdout) {
        Ok(Some((Msg::Hello { version }, _))) if version == VERSION => {}
        other => {
            let _ = child.kill();
            let _ = child.wait();
            // Most likely another justrust build replaced the machine's
            // binary: install ours again on the next attempt.
            if let Ok(d) = crate::remote::dir() {
                let _ = std::fs::remove_file(d.join("remote-justrust"));
            }
            bail!("remote agent handshake failed: {other:?}");
        }
    }
    let link = Arc::new(Link {
        child: Mutex::new(child),
        stdin: Mutex::new(stdin),
        routes: Mutex::default(),
        mirror: Mirror {
            target: Some(target.clone()),
            prefix: String::new(),
        },
    });
    {
        let link = link.clone();
        std::thread::spawn(move || read_agent(&link, stdout));
    }
    log(&format!("connected to {}", target.dest));
    st.link = Some(link.clone());
    d.set_info(|i| i.connected = true);
    // Everything the machine had may be stale: start from a full mirror.
    let roots: Vec<PathBuf> = st.roots.keys().cloned().collect();
    st.roots.clear();
    for r in roots {
        add_root(d, st, &link, &r)?;
    }
    Ok(link)
}

fn read_agent(link: &Link, mut stdout: BufReader<std::process::ChildStdout>) {
    while let Ok(Some((msg, payload))) = read_frame(&mut stdout) {
        let mut r = link.routes.lock().unwrap();
        match msg {
            Msg::BarrierAck { id, failed } => {
                if let Some(tx) = r.barriers.remove(&id) {
                    let _ = tx.send(failed);
                }
            }
            Msg::Out { id, .. } | Msg::Exit { id, .. } | Msg::Error { id, .. } if id != 0 => {
                let done = matches!(msg, Msg::Exit { .. } | Msg::Error { .. });
                if let Some(tx) = r.runs.get(&id) {
                    let _ = tx.send((msg, payload));
                }
                if done {
                    r.runs.remove(&id);
                }
            }
            Msg::Error { msg, .. } => log(&format!("agent: {msg}")),
            _ => {}
        }
    }
    // Connection lost: fail every waiter so clients fall back or stop.
    let mut r = link.routes.lock().unwrap();
    r.dead = true;
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
    log("agent connection closed");
}

fn update_root_info(d: &Daemon, st: &Sync) {
    let roots = st.roots.keys().map(|r| r.display().to_string()).collect();
    let skipped = st.roots.values().map(|r| r.oversized.len() as u64).sum();
    d.set_info(|i| {
        i.roots = roots;
        i.skipped = skipped;
    });
}

/// Watch every directory that holds a mirrored file (new ones included).
fn refresh_watches(d: &Daemon, root: &Path, m: &Manifest) {
    let n = d.watcher.watch_all(&remote_sync::dirs(root, m));
    if n > 0 {
        log(&format!("watching {n} more dirs under {}", root.display()));
    }
}

/// Start tracking a root: rsync it once, then remember its manifest.
fn add_root(d: &Daemon, st: &mut Sync, link: &Link, root: &Path) -> Result<()> {
    link.mirror.prepare(root)?;
    // Scan before rsync: a file edited during the copy then differs from
    // the manifest and is pushed again by the next flush.
    let scan = remote_sync::scan(root)?;
    let t = Instant::now();
    let n = link.mirror.rsync(root)?;
    link.send(
        &Msg::Synced {
            path: root.display().to_string(),
        },
        &[],
    )?;
    // rsync skips oversized files but leaves an older copy on the machine:
    // remove it so nothing builds on a stale version.
    for rel in &scan.oversized {
        link.send(
            &Msg::Delete {
                path: root.join(rel).display().to_string(),
            },
            &[],
        )?;
    }
    log(&format!(
        "mirrored {} ({} files sent) in {:.2}s",
        root.display(),
        n,
        t.elapsed().as_secs_f64()
    ));
    refresh_watches(d, root, &scan.files);
    st.roots.insert(
        root.to_path_buf(),
        Root {
            manifest: scan.files,
            oversized: scan.oversized,
        },
    );
    update_root_info(d, st);
    Ok(())
}

/// Push every difference between `root` and its manifest. Returns files sent.
fn push_root(d: &Daemon, st: &mut Sync, root: &Path) -> Result<usize> {
    let scan = remote_sync::scan(root)?;
    let mut new = scan.files;
    let diff = {
        let r = st.roots.get(root).context("unknown root")?;
        remote_sync::diff(&r.manifest, &new)
    };
    let link = st.link.clone().context("not connected")?;
    if !diff.is_empty() {
        if diff.changed.len() + diff.deleted.len() > remote_sync::RSYNC_ABOVE {
            // A branch switch or a big generated change: one rsync is cheaper.
            let t = Instant::now();
            let n = link.mirror.rsync(root)?;
            link.send(
                &Msg::Synced {
                    path: root.display().to_string(),
                },
                &[],
            )?;
            // `--max-size` skips oversized files without deleting an older
            // copy on the machine.
            for rel in &scan.oversized {
                link.send(
                    &Msg::Delete {
                        path: root.join(rel).display().to_string(),
                    },
                    &[],
                )?;
            }
            log(&format!(
                "rsynced {} ({} changed, {} deleted, {n} sent) in {:.2}s",
                root.display(),
                diff.changed.len(),
                diff.deleted.len(),
                t.elapsed().as_secs_f64()
            ));
        } else {
            for rel in &diff.changed {
                let p = root.join(rel);
                match std::fs::read(&p) {
                    Ok(data) if data.len() as u64 <= MAX_FILE => {
                        let mode = new.get(rel).map_or(0o644, |s| s.mode);
                        link.send(
                            &Msg::Write {
                                path: p.display().to_string(),
                                mode,
                            },
                            &data,
                        )?;
                    }
                    // Gone or grown since the scan: try again next time.
                    _ => {
                        new.insert(rel.clone(), UNKNOWN);
                    }
                }
            }
            for rel in &diff.deleted {
                link.send(
                    &Msg::Delete {
                        path: root.join(rel).display().to_string(),
                    },
                    &[],
                )?;
            }
        }
    }
    let n = diff.changed.len() + diff.deleted.len();
    refresh_watches(d, root, &new);
    if let Some(r) = st.roots.get_mut(root) {
        r.manifest = new;
        r.oversized = scan.oversized;
    }
    update_root_info(d, st);
    d.set_info(|i| i.pushed += n as u64);
    Ok(n)
}

/// Wait until the agent has applied everything sent so far. Paths the agent
/// failed to write or delete fail the barrier, and are marked so the next
/// push sends them again.
fn barrier(d: &Daemon, st: &mut Sync, link: &Link) -> Result<()> {
    let id = d.id();
    let (tx, rx) = channel();
    {
        let mut r = link.routes.lock().unwrap();
        if r.dead {
            bail!("connection to the remote machine was lost");
        }
        r.barriers.insert(id, tx);
    }
    link.send(&Msg::Barrier { id }, &[])?;
    let failed = rx
        .recv_timeout(BARRIER_TIMEOUT)
        .map_err(|_| anyhow::anyhow!("remote machine did not acknowledge the sync"))?;
    if failed.is_empty() {
        return Ok(());
    }
    for f in &failed {
        let f = Path::new(f);
        for (root, r) in st.roots.iter_mut() {
            if let Ok(rel) = f.strip_prefix(root) {
                // Present: rewrite it. Absent (a failed delete): the stale
                // entry makes the next diff delete it again.
                r.manifest.insert(rel.to_path_buf(), UNKNOWN);
            }
        }
    }
    bail!(
        "the remote machine could not apply {} file{} (see ~/.justrust/remote/daemon.log): {}",
        failed.len(),
        if failed.len() == 1 { "" } else { "s" },
        failed
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// A build source too large to mirror would make the remote build differ
/// from the local one: refuse with a clear message (the client builds
/// locally). Other large files (assets) are only reported.
fn oversized_note(st: &Sync, roots: &[PathBuf]) -> Result<String> {
    let mut src = Vec::new();
    let mut other = Vec::new();
    for root in roots {
        let Some(r) = st.roots.get(root) else {
            continue;
        };
        for rel in &r.oversized {
            let p = root.join(rel);
            let mb = std::fs::metadata(&p).map_or(0.0, |m| m.len() as f64 / 1048576.0);
            let s = format!("{} ({mb:.1} MB)", p.display());
            if remote_sync::is_build_source(rel) {
                src.push(s);
            } else {
                other.push(s);
            }
        }
    }
    if !src.is_empty() {
        bail!(
            "{} over the {} MB remote sync limit, not mirrored: {}",
            if src.len() == 1 {
                "a source file is"
            } else {
                "source files are"
            },
            MAX_FILE >> 20,
            src.join(", ")
        );
    }
    Ok(if other.is_empty() {
        String::new()
    } else {
        format!(
            "not mirrored (over {} MB): {}",
            MAX_FILE >> 20,
            other.join(", ")
        )
    })
}

#[allow(clippy::too_many_arguments)]
fn run(
    d: &Daemon,
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
    let roots: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
    let id = d.id();
    let (tx, rx) = channel();
    let (link, files, note) = {
        let mut st = d.sync.lock().unwrap();
        let res = (|| -> Result<_> {
            let link = ensure_link(d, &mut st, start)?;
            let mut files = 0;
            for root in &roots {
                if st.roots.contains_key(root) {
                    files += push_root(d, &mut st, root)?;
                } else {
                    add_root(d, &mut st, &link, root)?;
                }
            }
            barrier(d, &mut st, &link)?;
            let note = oversized_note(&st, &roots)?;
            {
                let mut rt = link.routes.lock().unwrap();
                if rt.dead {
                    bail!("connection to the remote machine was lost");
                }
                rt.runs.insert(id, tx);
            }
            link.send(
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
            Ok((link, files, note))
        })();
        match res {
            Ok(v) => v,
            Err(e) => {
                // A broken pipe or lost session: start over next time.
                if st.link.as_ref().is_some_and(|l| !l.alive()) {
                    drop_link(d, &mut st);
                }
                return Err(e);
            }
        }
    };
    let machine = d.info.lock().map(|i| i.machine.clone()).unwrap_or_default();
    write_frame(
        w,
        &Msg::Flushed {
            id,
            files,
            ms: t.elapsed().as_secs_f64() * 1000.0,
            machine,
            prefix: link.mirror.prefix.clone(),
            note,
        },
        &[],
    )?;
    // A client that sends Cancel or goes away (Ctrl+C, killed agent) cancels
    // the remote run, unless it already finished.
    let finished = Arc::new(AtomicBool::new(false));
    let cr = r.get_ref().try_clone()?;
    {
        let finished = finished.clone();
        let link = link.clone();
        std::thread::spawn(move || {
            let mut buf = BufReader::new(cr);
            let _ = read_frame(&mut buf);
            if !finished.load(Ordering::SeqCst) {
                let _ = link.send(&Msg::Cancel { id }, &[]);
            }
        });
    }
    while let Ok((msg, payload)) = rx.recv() {
        let done = matches!(msg, Msg::Exit { .. } | Msg::Error { .. });
        if done {
            finished.store(true, Ordering::SeqCst);
        }
        if write_frame(w, &msg, &payload).is_err() || done {
            break;
        }
    }
    Ok(())
}
