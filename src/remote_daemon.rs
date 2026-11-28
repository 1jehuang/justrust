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
//! the machine is in sync: after a reconnect a requested root is mirrored
//! again with rsync before it can build. Unrequested workspaces need not delay
//! it. A write the agent could not apply fails the next barrier.

use crate::sys::Detach;
use crate::sys::net::{UnixListener, UnixStream};
use anyhow::{Context, Result, bail};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufReader, BufWriter, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::peercache;
use crate::remote_backend::{self, Backend};
use crate::remote_checkpoint;
use crate::remote_proto::{Msg, VERSION, read_frame, write_frame};
use crate::remote_resume;
use crate::remote_sync::{self, MAX_FILE, Manifest, Mirror, UNKNOWN};
use crate::remote_watch::Watcher;

#[path = "remote_sync_queue.rs"]
mod sync_queue;
use sync_queue::SyncQueue;

const IDLE_EXIT: Duration = Duration::from_secs(3600);
const BARRIER_TIMEOUT: Duration = Duration::from_secs(60);

fn daemon_file(name: &str) -> Result<PathBuf> {
    let dir = crate::remote::dir()?;
    let platform = crate::remote_windows::selected_platform().unwrap_or("linux");
    if platform == "linux" {
        Ok(dir.join(name))
    } else {
        Ok(dir.join(format!("{platform}-{name}")))
    }
}

pub fn sock_path() -> Result<PathBuf> {
    daemon_file("daemon.sock")
}

/// Content identity, cached once so reinstalling identical bytes keeps the daemon.
pub fn build_id() -> String {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        #[cfg(unix)]
        let executable = crate::procfs::running_executable();
        #[cfg(not(unix))]
        let executable = std::env::current_exe().and_then(std::fs::File::open);
        let cache = crate::paths::home().ok().map(|home| home.join("build-id"));
        binary_identity(executable.and_then(|file| file_digest(cache.as_deref(), file)))
    })
    .clone()
}

/// Digest of an open executable, remembered across processes: every process
/// that names its build used to read its whole executable first (1.33 s for
/// the 174 MB debug client, 11 ms for the 20 MB release one).
///
/// The stored digest is only used for the same file in the same state:
/// device, inode, size, and modification and status-change times in
/// nanoseconds. Rewriting a file in place moves its status-change time even
/// when the modification time is put back, and an installer's replacement is
/// another inode. Anything else (no cache directory, no entry, another key,
/// a damaged entry) hashes the bytes, as before.
fn file_digest(cache: Option<&Path>, file: std::fs::File) -> std::io::Result<String> {
    let Some((cache, key)) = cache.zip(digest_key(&file)) else {
        return binary_digest(file);
    };
    let entry = cache.join(key.split(' ').take(2).collect::<Vec<_>>().join("-"));
    if let Some(digest) = stored_digest(&entry, &key) {
        return Ok(digest);
    }
    let digest = binary_digest(&file)?;
    // The file may have been written while it was read: only remember a
    // digest of a state that held from before the read until after it.
    if digest_key(&file).as_deref() == Some(&key) {
        store_digest(cache, &entry, &key, &digest);
    }
    Ok(digest)
}

/// `<device> <inode> <size> <mtime ns> <ctime ns>` of an open file.
#[cfg(unix)]
fn digest_key(file: &std::fs::File) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let m = file.metadata().ok()?;
    Some(format!(
        "{} {} {} {}.{:09} {}.{:09}",
        m.dev(),
        m.ino(),
        m.size(),
        m.mtime(),
        m.mtime_nsec(),
        m.ctime(),
        m.ctime_nsec()
    ))
}

#[cfg(not(unix))]
fn digest_key(_: &std::fs::File) -> Option<String> {
    None
}

fn stored_digest(entry: &Path, key: &str) -> Option<String> {
    let text = std::fs::read_to_string(entry).ok()?;
    let (stored_key, digest) = text.strip_suffix('\n')?.rsplit_once(' ')?;
    let hex = digest.strip_prefix("sha256-")?;
    let valid = hex.len() == 64
        && hex
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    (stored_key == key && valid).then(|| digest.to_owned())
}

/// Best effort: a digest that cannot be stored is computed again next time.
fn store_digest(cache: &Path, entry: &Path, key: &str, digest: &str) {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    if std::fs::create_dir_all(cache).is_err() {
        return;
    }
    let tmp = cache.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    // Readers see the old entry or the whole new one, never a partial line.
    if std::fs::write(&tmp, format!("{key} {digest}\n")).is_err()
        || std::fs::rename(&tmp, entry).is_err()
    {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    prune_digests(cache, entry, 256, Duration::from_secs(7 * 24 * 3600));
}

/// Every relinked development binary is a new inode with its own entry:
/// past `keep` entries, drop the ones not written for `older_than`.
fn prune_digests(cache: &Path, mine: &Path, keep: usize, older_than: Duration) {
    let Ok(rd) = std::fs::read_dir(cache) else {
        return;
    };
    let entries: Vec<_> = rd.flatten().collect();
    if entries.len() <= keep {
        return;
    }
    for e in entries {
        let old = e
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= older_than);
        if old && e.path() != mine {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

fn binary_digest(mut reader: impl Read) -> std::io::Result<String> {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    // Fixed memory regardless of binary size, one streaming pass per process.
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = match reader.read(&mut buffer) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("sha256-{:x}", hash.finalize()))
}

fn binary_identity(digest: std::io::Result<String>) -> String {
    digest.unwrap_or_else(|_| {
        // Do not share potentially incompatible binaries on read failure.
        // OnceLock freezes this conservative process-local fallback at first use.
        static NEXT: AtomicU64 = AtomicU64::new(0);
        format!(
            "unreadable-{}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
    })
}

/// Run/RunSync carries the build environment, not the daemon's connection
/// policy. Freeze that policy alongside the binary identity so a process
/// started with native=off cannot serve a later native=require client.
/// Keep this separate from build_id: artifact and session binary identities
/// must not change when only the local transport policy changes.
fn daemon_id() -> String {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| policy_identity(&build_id(), crate::remote_hosted::setting))
        .clone()
}

fn policy_identity(binary: &str, setting: impl Fn(&str) -> Option<String>) -> String {
    let mode = match setting("JUSTRUST_HOSTED_NATIVE").as_deref() {
        Some("1" | "on" | "true" | "require") => "required",
        Some("0" | "off" | "false") => "off",
        _ => "auto",
    };
    let only = mode != "off"
        && matches!(
            setting("JUSTRUST_HOSTED_NATIVE_ONLY").as_deref(),
            Some("1" | "on" | "true" | "require")
        );
    let isolated = hosted_mirror_isolation(setting("JUSTRUST_HOSTED_MIRROR_ISOLATION").as_deref());
    format!("{binary}:native={mode}:only={only}:mirror-isolation={isolated}")
}

// ------------------------------------------------------------------ client

/// Connect to the daemon, starting it if needed (or replacing a daemon from
/// another justrust build or hosted transport policy).
/// Response from `hosted_admit`: HTTP status, body, optional `Server-Timing`,
/// and daemon-measured round-trip latency in ms.
pub struct HostedAdmitResponse {
    pub code: u16,
    pub body: String,
    pub server_timing: Option<String>,
    pub ms: f64,
}

/// Send a hosted admission request (`POST /v1/build/runs`) through the running
/// daemon's persistent keep-alive HTTPS pool. Returns `Err` if the daemon
/// could not be reached before sending (safe to fall back to curl).
pub fn hosted_admit(
    key: &str,
    body: &str,
    timing: bool,
) -> Result<std::result::Result<HostedAdmitResponse, String>> {
    let sock = connect()?;
    let mut w = BufWriter::new(sock.try_clone()?);
    let mut r = BufReader::new(sock);
    write_frame(
        &mut w,
        &Msg::HostedAdmit {
            key: key.to_owned(),
            body: body.to_owned(),
            timing,
        },
        &[],
    )?;
    match read_frame(&mut r)? {
        Some((
            Msg::HostedAdmitResp {
                code,
                body,
                server_timing,
                error,
                ms,
            },
            _,
        )) => {
            if let Some(err) = error {
                Ok(Err(err))
            } else {
                Ok(Ok(HostedAdmitResponse {
                    code,
                    body,
                    server_timing,
                    ms,
                }))
            }
        }
        other => bail!("unexpected hosted admission daemon reply: {other:?}"),
    }
}

/// Push dirty source files over an already-connected SSH link while hosted
/// admission is in flight. Never wakes a stopped host and never starts a build.
pub fn pre_sync(roots: &[PathBuf]) -> Result<(usize, f64)> {
    let sock = connect()?;
    let mut w = BufWriter::new(sock.try_clone()?);
    let mut r = BufReader::new(sock);
    write_frame(
        &mut w,
        &Msg::PreSync {
            roots: roots.iter().map(|p| p.display().to_string()).collect(),
        },
        &[],
    )?;
    match read_frame(&mut r)? {
        Some((Msg::PreSyncDone { files, ms }, _)) => Ok((files, ms)),
        other => bail!("unexpected pre-sync daemon reply: {other:?}"),
    }
}

/// Fast non-spawning check: true only when a matching daemon is already running
/// with a live link and a recent clean hosted admission for `key`.
pub fn hosted_warm_ready(key: &str) -> bool {
    if key.is_empty() {
        return false;
    }
    let Ok(sock_file) = sock_path() else {
        return false;
    };
    let Ok(s) = UnixStream::connect(&sock_file) else {
        return false;
    };
    let me = policy_identity(&build_id(), crate::remote_hosted::setting);
    if !matches!(ping(&s), Ok(p) if p.build == me && p.connected) {
        return false;
    }
    let Ok(s) = UnixStream::connect(&sock_file) else {
        return false;
    };
    let Ok(w_stream) = s.try_clone() else {
        return false;
    };
    let mut w = BufWriter::new(w_stream);
    let mut r = BufReader::new(s);
    if write_frame(
        &mut w,
        &Msg::HostedWarmReady {
            key_hash: remote_checkpoint::digest(key.as_bytes()),
        },
        &[],
    )
    .is_err()
    {
        return false;
    }
    matches!(
        read_frame(&mut r),
        Ok(Some((Msg::HostedWarmReadyResp { ready: true }, _)))
    )
}

pub fn connect() -> Result<UnixStream> {
    let sock = sock_path()?;
    let me = policy_identity(&build_id(), crate::remote_hosted::setting);
    if let Ok(s) = UnixStream::connect(&sock) {
        match ping(&s) {
            // Retire incompatible daemons, but retain their active sessions.
            // No Run has been sent on this connection, so replacement cannot
            // replay a build or make its execution status ambiguous.
            Ok(p) if p.build == me => return Ok(s),
            _ => shutdown_and_wait(&s),
        }
    }
    // Shutdown's socket read can time out while the old daemon is still
    // saving manifests. Starting now would make the replacement lose the
    // lock race and exit, leaving us polling a socket nobody will bind.
    // Verify retirement through the same lock the daemon uses, not EOF or
    // socket disappearance. Never unlink a socket while its owner holds it.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(daemon_file("daemon.lock")?)?;
    if let Some(s) = wait_for_daemon_lock(
        &lock,
        Duration::from_secs(30),
        || {
            let s = UnixStream::connect(&sock).ok()?;
            ping(&s).is_ok_and(|p| p.build == me).then_some(s)
        },
        || std::thread::sleep(Duration::from_millis(20)),
    )? {
        return Ok(s);
    }
    drop(lock);
    spawn()?;
    let t = Instant::now();
    loop {
        if let Ok(s) = UnixStream::connect(&sock)
            && ping(&s).is_ok_and(|p| p.build == me)
        {
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
    // macOS fails setsockopt with EINVAL once the peer has closed. That must
    // not turn a pong already read into "daemon not running". On a live
    // socket this cannot fail, and a closed one fails its next read anyway.
    let _ = s.set_read_timeout(None);
    res
}

fn ping(s: &UnixStream) -> Result<Pong> {
    ping_with(s, Duration::from_secs(2))
}

/// Wait for exclusive ownership before spawning a replacement. A socket
/// timeout is not proof that the old daemon has finished saving manifests.
fn wait_for_daemon_lock(
    lock: &std::fs::File,
    timeout: Duration,
    mut ready: impl FnMut() -> Option<UnixStream>,
    mut wait: impl FnMut(),
) -> Result<Option<UnixStream>> {
    let start = Instant::now();
    loop {
        // Another client may have already started our replacement. Its
        // healthy socket is sufficient, even though it now owns the lock.
        if let Some(s) = ready() {
            return Ok(Some(s));
        }
        match lock.try_lock() {
            Ok(()) => return Ok(None),
            Err(std::fs::TryLockError::WouldBlock) => {
                if start.elapsed() >= timeout {
                    bail!(
                        "sync daemon shutdown still holds daemon.lock after {timeout:?} (see ~/.justrust/remote/daemon.log)"
                    );
                }
                wait();
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(error).context("waiting for sync daemon shutdown lock");
            }
        }
    }
}

/// Ask a daemon to exit and wait for its socket to close, up to the read
/// timeout. Callers starting a replacement must also verify lock handoff.
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
        // Long-lived and shared by every build: per-build switches of the
        // build that happened to start it must not stick. Builds still opt
        // out on their own (their shims check `JUSTRUST_PEER_CACHE`).
        .env_remove("JUSTRUST_PEER_CACHE")
        .env_remove("JUSTRUST_DEPCACHE")
        .env_remove("JUSTRUST_DEPCACHE_DIR")
        .env_remove("JUSTRUST_DEPCACHE_EXPLAIN")
        .stdin(Stdio::null())
        .stdout(log.try_clone()?)
        .stderr(log);
    // Own session: survives the agent's terminal and its Ctrl+C.
    c.detached();
    crate::procfs::close_inherited_fds(&mut c);
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
    barriers: HashMap<u64, Sender<SyncAck>>,
    /// `Verify` answers: whether the checkpoint held, else why not.
    verifies: HashMap<u64, Sender<Result<(), String>>>,
    runs: HashMap<u64, Sender<(Msg, Vec<u8>)>>,
}

/// One ssh session to the agent.
struct Link {
    child: Mutex<Child>,
    stdin: Arc<Mutex<BufWriter<ChildStdin>>>,
    routes: Mutex<Routes>,
    mirror: Mirror,
    /// Transport drops this link's durable session reattached through.
    outage: Arc<Outage>,
    /// Peer dependency cache over this session (`src/peercache.rs`).
    peer: Arc<peercache::Relay>,
    /// The agent serves `Checkpoint` and `Verify`, and this daemon uses them.
    checkpoint: bool,
}

impl Link {
    fn alive(&self) -> bool {
        !self.routes.lock().unwrap().dead
    }

    fn send(&self, m: &Msg, payload: &[u8]) -> Result<()> {
        let result = write_frame(&mut *self.stdin.lock().unwrap(), m, payload);
        if result.is_err() {
            self.routes.lock().unwrap().dead = true;
        }
        result
    }

    fn kill(&self) {
        let mut c = self.child.lock().unwrap();
        let _ = c.kill();
        let _ = c.wait();
    }
}

struct Root {
    /// True exactly when the committed manifest contains an UNKNOWN stamp.
    pending_repair: bool,
    manifest: Manifest,
    /// Derived only from manifest paths. Watches are still retried every flush.
    watch_dirs: Option<std::collections::BTreeSet<PathBuf>>,
    /// Files over `MAX_FILE`, not mirrored.
    oversized: Vec<PathBuf>,
    /// A new agent connection cannot trust the previous connection's manifest.
    /// Resync on demand, not all remembered workspaces before the first build.
    needs_full_sync: bool,
    /// Evidence that survives a reconnect (`remote_checkpoint`).
    mark: remote_checkpoint::Mark,
    /// `(link_ptr, watcher_event_gen)` recorded when a scan + push completed
    /// with all directories already watched and no inotify events since.
    /// Allows `push_root` to skip rescanning large trees while the watcher is quiet.
    clean_event_gen: Option<(usize, u64)>,
}

impl Root {
    fn watch_directories(
        &mut self,
        root: &Path,
        new: &Manifest,
        diff: &remote_sync::Diff,
    ) -> &std::collections::BTreeSet<PathBuf> {
        // Stamps do not affect directory membership. A failed watch, renamed
        // inode or replaced directory still gets retried by watch_all, using
        // this cached path set. Never skip source scanning or the sync barrier.
        let topology_changed = !diff.deleted.is_empty()
            || diff
                .changed
                .iter()
                .any(|path| !self.manifest.contains_key(path));
        if self.watch_dirs.is_none() || topology_changed {
            self.watch_dirs = Some(remote_sync::dirs(root, new));
        }
        self.watch_dirs.as_ref().unwrap()
    }
}

/// Capacity failures survive individual requests, not daemon restarts. The
/// receiver may be repaired or replaced, so never turn one into a permanent ban.
#[derive(Default)]
struct InstallRetry(Option<(String, Instant, String)>);

impl InstallRetry {
    fn run<T>(
        &mut self,
        dest: &str,
        now: impl Fn() -> Instant,
        install: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        if let Some((failed, until, diagnostic)) = &self.0
            && failed == dest
            && let Some(left) = until.checked_duration_since(now())
            && !left.is_zero()
        {
            bail!(
                "remote executable installation paused for {}s on {dest}; free disk space or quota on the build machine: {diagnostic}",
                left.as_secs() + u64::from(left.subsec_nanos() != 0)
            );
        }
        self.0 = None;
        let result = install();
        if let Err(err) = &result
            && err.is::<remote_sync::InstallDiskFull>()
        {
            self.0 = Some((
                dest.to_owned(),
                now() + Duration::from_secs(60),
                format!("{err:#}"),
            ));
        }
        result
    }
}

/// After a failed handshake: must the machine's justrust be installed again?
///
/// Only an agent that answered in another protocol version shows that some
/// other build replaced the binary. No answer (a dropped link, a closed
/// stream) used to count as that too, so every drop cost a full install over
/// the link that had just failed: seven in half an hour on 2026-11-27, 28 to
/// 165 s each against 0.7 s for a handshake. Now the machine is asked, in one
/// command, and its binary is replaced only if it says it is another one.
fn stamp_is_stale(answered: Option<u32>, intact: impl FnOnce() -> Option<bool>) -> bool {
    match answered {
        Some(version) => version != VERSION,
        None => intact() == Some(false),
    }
}

/// How long a link's transport was down, from its session bridge's stderr.
/// The bridge hides a reattached drop from the frame stream, so without this
/// a build only looks slow to return.
#[derive(Default)]
struct Outage {
    restored_us: AtomicU64,
    down: Mutex<Option<Instant>>,
}

impl Outage {
    /// Feed one stderr line of the bridge.
    fn observe(&self, line: &str) {
        if line.starts_with(crate::remote_proto::INTERRUPTED) {
            self.down.lock().unwrap().get_or_insert_with(Instant::now);
        } else if let Some(secs) = crate::remote_proto::restored_secs(line) {
            self.restored_us
                .fetch_add((secs * 1e6) as u64, Ordering::SeqCst);
            *self.down.lock().unwrap() = None;
        }
    }

    /// The bridge reported a drop it has not recovered from.
    fn is_down(&self) -> bool {
        self.down.lock().unwrap().is_some()
    }

    /// Milliseconds of outages that ended in a reattach.
    fn restored_ms(&self) -> f64 {
        self.restored_us.load(Ordering::SeqCst) as f64 / 1000.0
    }

    /// As `restored_ms`, for a caller holding a frame that arrived after a
    /// reattach: stderr is read on another thread, so give its line a moment.
    fn settled_ms(&self) -> f64 {
        let waited = Instant::now();
        while self.is_down() && waited.elapsed() < Duration::from_millis(100) {
            std::thread::sleep(Duration::from_millis(2));
        }
        self.restored_ms()
    }
}

/// Everything network operations touch. Held across them.
struct Sync {
    install_retry: InstallRetry,
    /// Where the last connect's time went, until the run that paid reports it.
    link_parts: Option<[Duration; 3]>,
    /// Roots of recent builds, newest last: re-verified after a relink.
    recent_roots: Vec<(PathBuf, Instant)>,
    // One whole-root-set receipt, never persisted or shared across connections.
    receipt: Option<(Vec<std::ffi::OsString>, String)>,
    backend: Backend,
    link: Option<Arc<Link>>,
    roots: BTreeMap<PathBuf, Root>,
}

impl Sync {
    /// Agent barriers cover the whole connection, not just a run's roots.
    /// A rejected write in an idle workspace must therefore be resent even
    /// when the next build asks for a different workspace.
    fn repair_roots(&self) -> Vec<PathBuf> {
        self.roots
            .iter()
            .filter(|(_, r)| r.pending_repair)
            .map(|(root, _)| root.clone())
            .collect()
    }

    fn needs_mirror(&self, root: &Path) -> bool {
        self.roots.get(root).is_none_or(|r| r.needs_full_sync)
    }

    /// A partial copy invalidates this workspace, not the shared SSH session.
    fn source_sync_failed(&mut self, root: &Path) {
        self.receipt = None;
        if let Some(r) = self.roots.get_mut(root) {
            r.needs_full_sync = true;
            r.clean_event_gen = None;
        }
    }

    fn invalidate_sources(&mut self) {
        self.receipt = None;
        self.roots.retain(|r, root| {
            if !r.is_dir() {
                log(&format!("forgot {} (deleted)", r.display()));
                return false;
            }
            root.needs_full_sync = true;
            root.clean_event_gen = None;
            true
        });
    }
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

impl Info {
    /// Only fields displayed by `remote status` invalidate its snapshot.
    fn update(&mut self, f: impl FnOnce(&mut Self)) -> bool {
        let before = (
            self.roots.clone(),
            self.machine.clone(),
            self.pushed,
            self.connected,
        );
        f(self);
        before.0 != self.roots
            || before.1 != self.machine
            || before.2 != self.pushed
            || before.3 != self.connected
    }
}

/// A lossy wakeup, not persisted state. Never wait on a subscriber or perform
/// filesystem IO on the build/sync thread. One pending wakeup is sufficient.
fn status_hints(path: PathBuf) -> std::sync::mpsc::SyncSender<()> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let _ = std::thread::Builder::new()
        .name("daemon-status".into())
        .spawn(move || {
            while rx.recv().is_ok() {
                write_status_hint(&path);
            }
        });
    tx
}

fn write_status_hint(path: &Path) {
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
        // Do not follow symlinks, block on a FIFO, or modify an unrelated inode.
        // Reuse the inode so directory watchers receive CLOSE_WRITE each time.
        let write = || -> std::io::Result<()> {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(path)?;
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != unsafe { libc::geteuid() }
                || metadata.nlink() != 1
            {
                return Ok(());
            }
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            file.set_len(0)?;
            file.write_all(b"1\n")
        };
        let _ = write();
    }
    #[cfg(not(unix))]
    let _ = path;
}

struct PreSyncStamp {
    link_ptr: usize,
    at: Instant,
    event_gen: u64,
    files: usize,
}

const WARM_ADMIT_TTL: Duration = Duration::from_secs(120);

struct WarmAdmit {
    key_hash: String,
    link_ptr: usize,
    at: Instant,
}

const LEASE_BATCH_COUNT: u64 = 16;
const LEASE_SAFETY_MARGIN: Duration = Duration::from_secs(10);
const LEASE_REFILL_REMAINING: u64 = 4;
const LEASE_REFILL_BEFORE_EXPIRY: Duration = Duration::from_secs(30);
const LEASE_RETRY_BACKOFF: Duration = Duration::from_secs(30);

struct BuildLease {
    lease_id: String,
    key: String,
    key_hash: String,
    /// `0` means issued during Build 1 before initial connect; bound to the first live link on checkout.
    link_ptr: usize,
    granted: u64,
    used: u64,
    expires_at: Instant,
    builds: serde_json::Value,
    fleet_version: Option<String>,
    pending_runs: Vec<serde_json::Value>,
}

impl BuildLease {
    fn usable_for(&self, key_hash: &str, live_link_ptr: usize, now: Instant) -> bool {
        live_link_ptr != 0
            && self.key_hash == key_hash
            && (self.link_ptr == 0 || self.link_ptr == live_link_ptr)
            && self.used < self.granted
            && now + LEASE_SAFETY_MARGIN < self.expires_at
    }

    fn needs_refill(&self, now: Instant) -> bool {
        self.granted.saturating_sub(self.used) <= LEASE_REFILL_REMAINING
            || self.expires_at.saturating_duration_since(now) <= LEASE_REFILL_BEFORE_EXPIRY
    }
}

#[derive(Default)]
struct LeasePool {
    active: Option<BuildLease>,
    draining: Vec<BuildLease>,
    refill_in_flight: bool,
    redeem_in_flight: bool,
    retry_after: Option<Instant>,
}

struct Daemon {
    sync: SyncQueue<Sync>,
    info: Mutex<Info>,
    status_hint: Option<std::sync::mpsc::SyncSender<()>>,
    next_id: AtomicU64,
    watcher: Watcher,
    /// Device and inode of our socket, to remove it on exit only if it is
    /// still ours.
    sock: (PathBuf, (u64, u64)),
    /// The daemon lock. Released when draining so a replacement can start.
    lock: Mutex<Option<std::fs::File>>,
    /// Shut down but still relaying runs that were in flight.
    draining: AtomicBool,
    /// The relay of the current link, for this machine's rustc shims.
    peer: peercache::Current,
    /// Our `peer.sock` (path, device, inode), removed on exit if still ours.
    peer_sock: Mutex<Option<(PathBuf, (u64, u64))>>,
    /// Persistent keep-alive HTTPS client for hosted admission (`POST /v1/build/runs`).
    hosted_agent: ureq::Agent,
    /// Roots recently scanned and pushed by `Msg::PreSync` during admission.
    pre_synced: Mutex<BTreeMap<PathBuf, PreSyncStamp>>,
    /// Current live `Link` pointer (0 when disconnected), lock-free for fast readiness checks.
    live_link_ptr: AtomicUsize,
    /// Last clean `200 Charged` hosted admission with >=2 builds remaining on the live link.
    warm_admit: Mutex<Option<WarmAdmit>>,
    /// Pre-authorized monthly included build-allowance leases for ~0 ms warm admission.
    leases: Arc<Mutex<LeasePool>>,
    /// Wakes `relink_loop` when a link died of a transport drop.
    relink: Option<std::sync::mpsc::SyncSender<()>>,
}

fn hosted_admit_agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .max_redirects(0)
        .max_redirects_will_error(false)
        .timeout_resolve(Some(Duration::from_secs(10)))
        .timeout_connect(Some(Duration::from_secs(10)))
        .timeout_global(Some(Duration::from_secs(30)))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .new_agent()
}

impl Daemon {
    fn id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    fn set_info(&self, f: impl FnOnce(&mut Info)) {
        let changed = self.info.lock().is_ok_and(|mut i| i.update(f));
        if changed && let Some(tx) = &self.status_hint {
            let _ = tx.try_send(());
        }
    }

    /// Stop being the daemon: remove our socket (if a replacement has not
    /// already bound a new one) and release the lock.
    fn retire(&self) {
        let (p, id) = &self.sock;
        if std::fs::metadata(p).is_ok_and(|m| crate::sys::file_id(&m) == *id) {
            let _ = std::fs::remove_file(p);
        }
        if let Some((p, id)) = self.peer_sock.lock().ok().and_then(|mut s| s.take())
            && std::fs::metadata(&p).is_ok_and(|m| crate::sys::file_id(&m) == id)
        {
            let _ = std::fs::remove_file(&p);
        }
        if let Ok(mut l) = self.lock.lock()
            && let Some(lock) = l.take()
        {
            // Closing our descriptor alone need not release the lock while a
            // concurrently spawned child still holds an inherited descriptor.
            // Explicitly unlock the shared open-file description before a
            // replacement daemon attempts to acquire it.
            if let Err(error) = lock.unlock() {
                log(&format!("daemon lock release failed: {error}"));
            }
        }
    }

    fn exit(&self, why: &str) -> ! {
        log(why);
        // Draining: already saved before retiring (a replacement may be
        // mirroring now, and must not find a token written after it began).
        if !self.draining.load(Ordering::SeqCst) {
            save_mirrors(self);
        }
        flush_and_close_leases_on_exit(&self.hosted_agent, &self.leases);
        self.retire();
        std::process::exit(0)
    }

    /// `Shutdown` (a newer build, backend or transport policy): retire at once so a
    /// replacement can start, but let runs in flight finish streaming
    /// rather than cutting off another agent's build.
    fn shutdown(self: &Arc<Self>) {
        let active = self.info.lock().map_or(0, |i| i.active);
        if active == 0 {
            self.exit("shutdown requested");
        }
        if !self.begin_draining(active) {
            return;
        }
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

    fn begin_draining(&self, active: u64) -> bool {
        if self.draining.swap(true, Ordering::SeqCst) {
            return false;
        }
        log(&format!(
            "shutdown requested, finishing {active} run(s) in flight"
        ));
        // Still the only daemon (we hold the lock): save now, while no
        // replacement can be writing to the machine.
        save_mirrors(self);
        self.retire();
        true
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
    // Long-lived: never pin the cwd of whichever (possibly temporary)
    // worktree spawned us. Once it is deleted, every child rsync fails with
    // "getcwd(): No such file or directory".
    let _ = std::env::set_current_dir("/");
    let backend = remote_backend::load().context("no remote backend configured")?;
    let sock = sock_path()?;
    // Exclusive: a second daemon exits instead of stealing the socket.
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(daemon_file("daemon.lock")?)?;
    if lock.try_lock().is_err() {
        bail!("another daemon is running");
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;
    }
    let md = std::fs::metadata(&sock)?;
    log(&format!(
        "daemon up, backend {}, build {}",
        backend.label(),
        daemon_id()
    ));
    let (wtx, wrx) = channel::<PathBuf>();
    let (relink_tx, relink_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let d = Arc::new(Daemon {
        status_hint: Some(status_hints(sock.with_file_name("daemon-status"))),
        info: Mutex::new(Info {
            machine: backend.label(),
            last_client: Some(Instant::now()),
            ..Default::default()
        }),
        sync: SyncQueue::new(Sync {
            install_retry: Default::default(),
            link_parts: None,
            recent_roots: Vec::new(),
            receipt: None,
            backend,
            link: None,
            roots: BTreeMap::new(),
        }),
        relink: Some(relink_tx),
        next_id: AtomicU64::new(1),
        watcher: Watcher::new(wtx)?,
        sock: (sock.clone(), crate::sys::file_id(&md)),
        lock: Mutex::new(Some(lock)),
        draining: AtomicBool::new(false),
        peer: Arc::default(),
        peer_sock: Mutex::default(),
        hosted_agent: hosted_admit_agent(),
        pre_synced: Mutex::default(),
        live_link_ptr: AtomicUsize::new(0),
        warm_admit: Mutex::default(),
        leases: Arc::default(),
    });
    if peercache::enabled()
        && let Some(p) = peercache::sock_path()
    {
        match peercache::listen(&p, d.peer.clone()) {
            Ok(()) => {
                if let (Ok(m), Ok(mut s)) = (std::fs::metadata(&p), d.peer_sock.lock()) {
                    *s = Some((p, crate::sys::file_id(&m)));
                }
            }
            Err(e) => log(&format!("peer cache socket {}: {e}", p.display())),
        }
    } else {
        log("peer cache off (JUSTRUST_PEER_CACHE=0 or JUSTRUST_DEPCACHE=0)");
    }
    {
        let d = d.clone();
        std::thread::spawn(move || peer_sync_loop(&d));
    }
    {
        let d = d.clone();
        std::thread::spawn(move || push_loop(&d, wrx));
    }
    {
        let d = d.clone();
        std::thread::spawn(move || relink_loop(&d, relink_rx));
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

#[path = "remote_daemon_debounce.rs"]
mod debounce;

/// After a transport drop killed the link, connect again and re-verify the
/// mirrors of recent builds, so the next build does not wait for either.
/// Bounded: only while a client used this daemon recently, a few attempts,
/// and never starting a stopped machine. Builds always go first.
fn relink_loop(d: &Daemon, rx: Receiver<()>) {
    const ATTEMPTS: [u64; 6] = [1, 2, 4, 8, 15, 30];
    const RECENT: Duration = Duration::from_secs(600);
    while rx.recv().is_ok() {
        for wait in ATTEMPTS {
            std::thread::sleep(Duration::from_secs(wait));
            let used = d
                .info
                .lock()
                .is_ok_and(|i| i.active > 0 || i.last_client.is_some_and(|t| t.elapsed() < RECENT));
            if d.draining.load(Ordering::SeqCst) || !used {
                break;
            }
            // A build holding or waiting for the lock connects by itself.
            let Ok(st) = d.sync.try_background_lock() else {
                continue;
            };
            if st.link.as_ref().is_some_and(|l| l.alive()) {
                break;
            }
            let t = Instant::now();
            let (mut st, link) = ensure_link(d, st, false, Relock::Background);
            let link = match link {
                Ok(link) => link,
                Err(e) => {
                    log(&format!("relink: {e:#}"));
                    continue;
                }
            };
            // No run waited for this connect: none should report its parts.
            st.link_parts = None;
            let roots: Vec<PathBuf> = st
                .recent_roots
                .iter()
                .rev()
                .filter(|(root, at)| at.elapsed() < RECENT && root.is_dir())
                .take(2)
                .map(|(root, _)| root.clone())
                .collect();
            let mut verified = 0;
            for root in &roots {
                if d.sync.foreground_waiting() || !link.alive() {
                    break;
                }
                if !st.needs_mirror(root) {
                    continue;
                }
                match mirror_root(d, &mut st, &link, root) {
                    Ok(_) => verified += 1,
                    Err(e) => {
                        st.source_sync_failed(root);
                        log(&format!("relink: mirror {}: {e:#}", root.display()));
                    }
                }
            }
            log(&format!(
                "relinked after a transport drop in {:.2}s, {verified} of {} recent mirror(s) re-verified",
                t.elapsed().as_secs_f64(),
                roots.len()
            ));
            break;
        }
        // Drops during the attempts are covered by them.
        while rx.try_recv().is_ok() {}
    }
}

/// Debounce watch events, then push the touched roots.
fn push_loop(d: &Daemon, rx: Receiver<PathBuf>) {
    while let Ok(first) = rx.recv() {
        let touched = debounce::collect(first, &rx);
        if d.draining.load(Ordering::SeqCst) {
            continue;
        }
        let (roots, mut epoch, mut observed_link) = {
            let Ok(st) = d.sync.background_lock() else {
                return;
            };
            if !st.link.as_ref().is_some_and(|l| l.alive()) {
                continue;
            }
            let roots: Vec<PathBuf> = st
                .roots
                .keys()
                .filter(|r| {
                    !st.roots[*r].needs_full_sync && touched.iter().any(|t| t.starts_with(r))
                })
                .cloned()
                .collect();
            (roots, d.sync.foreground_epoch(), st.link.clone().unwrap())
        };
        'roots: for r in roots {
            // Fast path: if the root already has a clean generation and the
            // watcher has exact dirty paths, push incrementally without a full scan.
            let can_incremental = {
                let Ok(st) = d.sync.background_lock() else {
                    return;
                };
                !d.draining.load(Ordering::SeqCst)
                    && st.link.as_ref().is_some_and(|link| {
                        link.alive()
                            && st.roots.get(&r).is_some_and(|root| {
                                !root.pending_repair
                                    && !root.needs_full_sync
                                    && root.clean_event_gen.is_some_and(|(lp, event_gen)| {
                                        lp == Arc::as_ptr(link) as usize
                                            && d.watcher.dirty_since(&r, event_gen).is_some()
                                    })
                            })
                    })
            };
            if can_incremental {
                let Ok(mut st) = d.sync.background_lock() else {
                    return;
                };
                if d.draining.load(Ordering::SeqCst) || !st.link.as_ref().is_some_and(|l| l.alive())
                {
                    break 'roots;
                }
                if !st.roots.get(&r).is_some_and(|root| !root.needs_full_sync) {
                    continue 'roots;
                }
                if !r.is_dir() {
                    st.receipt = None;
                    st.roots.remove(&r);
                    log(&format!("forgot {} (deleted)", r.display()));
                    continue 'roots;
                }
                if let Err(e) = push_root(d, &mut st, &r) {
                    log(&format!("push {}: {e:#}", r.display()));
                    st.source_sync_failed(&r);
                    if st.link.as_ref().is_some_and(|l| !l.alive()) {
                        drop_link(d, &mut st);
                        break 'roots;
                    }
                }
                continue 'roots;
            }
            // One unlocked retry avoids holding up the next build on the first
            // conflict. A second conflict takes the existing locked fallback:
            // repeated foreground changes cannot restart this root forever.
            let mut retried = false;
            loop {
                // Only discovery is speculative. No mirror operation runs unlocked.
                let event_gen = d.watcher.event_count();
                let total = Instant::now();
                let scan = remote_sync::scan(&r);
                let scanned = total.elapsed();
                // One complete root remains atomic. Yield the mirror lock between
                // roots so a queued foreground build need not wait for the batch.
                let Ok(mut st) = d.sync.background_lock() else {
                    return;
                };
                if d.draining.load(Ordering::SeqCst) || !st.link.as_ref().is_some_and(|l| l.alive())
                {
                    break 'roots;
                }
                // A foreground sync may have repaired, replaced or removed this
                // root while the watcher yielded. Recheck under the same lock.
                if !st.roots.get(&r).is_some_and(|root| !root.needs_full_sync) {
                    continue 'roots;
                }
                if !r.is_dir() {
                    // Deleted (a removed worktree): forget it instead of
                    // dropping the link over a failed rsync.
                    st.receipt = None;
                    st.roots.remove(&r);
                    log(&format!("forgot {} (deleted)", r.display()));
                    continue 'roots;
                }
                let result = if epoch == d.sync.foreground_epoch()
                    && st
                        .link
                        .as_ref()
                        .is_some_and(|link| Arc::ptr_eq(link, &observed_link))
                {
                    scan.and_then(|scan| {
                        push_scanned(d, &mut st, &r, scan, total, scanned, event_gen)
                    })
                } else {
                    // Never compare an older scan to a newer foreground manifest.
                    epoch = d.sync.foreground_epoch();
                    observed_link = st.link.clone().unwrap();
                    if !retried {
                        retried = true;
                        drop(st);
                        continue;
                    }
                    push_root(d, &mut st, &r)
                };
                if let Err(e) = result {
                    log(&format!("push {}: {e:#}", r.display()));
                    st.source_sync_failed(&r);
                    if st.link.as_ref().is_some_and(|l| !l.alive()) {
                        drop_link(d, &mut st);
                        break 'roots;
                    }
                }
                break;
            }
        }
    }
}

fn client(stream: UnixStream, d: &Arc<Daemon>) -> Result<()> {
    let mut r = BufReader::new(stream.try_clone()?);
    let mut w = BufWriter::new(stream.try_clone()?);
    while let Some((msg, _)) = read_frame(&mut r)? {
        // Status pings (waybar polls every few seconds) must not keep an
        // otherwise idle daemon alive.
        if !matches!(msg, Msg::Ping) {
            d.set_info(|i| i.last_client = Some(Instant::now()));
        }
        let pipeline = matches!(&msg, Msg::RunSync { .. });
        match msg {
            Msg::Ping => {
                // `connected` is only updated on connect/drop: a link the
                // agent closed (machine stopped when idle) is not connected.
                let alive = d
                    .sync
                    .try_lock()
                    .ok()
                    .map(|s| s.link.as_ref().is_some_and(|l| l.alive()));
                let pong = {
                    let i = d.info.lock().unwrap();
                    Msg::Pong {
                        roots: i.roots.clone(),
                        machine: i.machine.clone(),
                        pushed: i.pushed,
                        connected: alive.unwrap_or(i.connected),
                        build: daemon_id(),
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
            Msg::Warm => {
                // Connect in the background so this machine's shims can use
                // the peer cache. Never starts a stopped machine.
                let d = d.clone();
                std::thread::spawn(move || {
                    peercache::CONNECTING.store(true, Ordering::SeqCst);
                    if let Ok(st) = d.sync.try_background_lock()
                        && let (_, Err(e)) = ensure_link(&d, st, false, Relock::Background)
                    {
                        log(&format!("warm: {e:#}"));
                    }
                    peercache::CONNECTING.store(false, Ordering::SeqCst);
                });
                return Ok(());
            }
            Msg::HostedAdmit { key, body, timing } => {
                let t = Instant::now();
                if let Some(leased_body) = try_checkout_build_lease(d, &key, &body) {
                    let resp = Msg::HostedAdmitResp {
                        code: 200,
                        body: leased_body,
                        server_timing: Some("jr_lease;dur=0.1".to_owned()),
                        error: None,
                        ms: t.elapsed().as_secs_f64() * 1000.0,
                    };
                    write_frame(&mut w, &resp, &[])?;
                    return Ok(());
                }
                let url = format!("{}/build/runs", crate::remote_hosted::DEFAULT_BUILD_BASE);
                let mut req = d
                    .hosted_agent
                    .post(&url)
                    .header("Authorization", &format!("Bearer {key}"))
                    .header("Content-Type", "application/json")
                    .header("X-Justrust-Client-Version", crate::update::VERSION)
                    .header("X-Justrust-Admission", "maintenance-v1");
                if let Some(target) = crate::update::target() {
                    req = req.header("X-Justrust-Target", target);
                }
                if timing {
                    req = req
                        .header("X-Justrust-Timing", "1")
                        .header("X-Builds-Timing", "1");
                }
                let resp = match req.send(body.as_bytes()) {
                    Ok(mut r) => {
                        let code = r.status().as_u16();
                        let server_timing = r
                            .headers()
                            .get("server-timing")
                            .and_then(|v| v.to_str().ok())
                            .map(str::to_owned);
                        match r
                            .body_mut()
                            .with_config()
                            .limit(1024 * 1024)
                            .read_to_string()
                        {
                            Ok(text) => {
                                record_warm_admit(d, &key, code, &text);
                                if code == 200 {
                                    maybe_warm_build_lease(d, &key, &text);
                                } else {
                                    invalidate_active_lease(d);
                                }
                                Msg::HostedAdmitResp {
                                    code,
                                    body: text,
                                    server_timing,
                                    error: None,
                                    ms: t.elapsed().as_secs_f64() * 1000.0,
                                }
                            }
                            Err(e) => {
                                *d.warm_admit.lock().unwrap() = None;
                                invalidate_active_lease(d);
                                Msg::HostedAdmitResp {
                                    code: 0,
                                    body: String::new(),
                                    server_timing: None,
                                    error: Some(format!("hosted admission read failed: {e}")),
                                    ms: t.elapsed().as_secs_f64() * 1000.0,
                                }
                            }
                        }
                    }
                    Err(e) => {
                        *d.warm_admit.lock().unwrap() = None;
                        invalidate_active_lease(d);
                        Msg::HostedAdmitResp {
                            code: 0,
                            body: String::new(),
                            server_timing: None,
                            error: Some(format!("hosted admission failed: {e}")),
                            ms: t.elapsed().as_secs_f64() * 1000.0,
                        }
                    }
                };
                write_frame(&mut w, &resp, &[])?;
                return Ok(());
            }
            Msg::HostedWarmReady { key_hash } => {
                let link_ptr = d.live_link_ptr.load(Ordering::SeqCst);
                let connected = d.info.lock().is_ok_and(|i| i.connected);
                let now = Instant::now();
                let lease_ready = d
                    .leases
                    .lock()
                    .unwrap()
                    .active
                    .as_ref()
                    .is_some_and(|l| l.usable_for(&key_hash, link_ptr, now));
                let ready = !d.draining.load(Ordering::SeqCst)
                    && connected
                    && link_ptr != 0
                    && (lease_ready
                        || d.warm_admit.lock().unwrap().as_ref().is_some_and(|wa| {
                            wa.link_ptr == link_ptr
                                && wa.key_hash == key_hash
                                && wa.at.elapsed() < WARM_ADMIT_TTL
                        }));
                write_frame(&mut w, &Msg::HostedWarmReadyResp { ready }, &[])?;
                return Ok(());
            }
            Msg::PreSync { roots } => {
                let t = Instant::now();
                let roots: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
                let mut files = 0usize;
                // Speculative pre-sync must never queue behind an active or
                // waiting foreground build: otherwise concurrent builds double-
                // queue (once for PreSync, then again for RunSync).
                if let Ok(mut st) = d.sync.try_background_lock()
                    && !d.draining.load(Ordering::SeqCst)
                    && let Some(link) = st.link.as_ref().filter(|l| l.alive()).cloned()
                {
                    if let Ok(n) = repair_sources(d, &mut st, &link) {
                        files += n;
                    }
                    for root in &roots {
                        let event_gen = d.watcher.event_count();
                        let result = if st.needs_mirror(root) {
                            mirror_root(d, &mut st, &link, root)
                        } else {
                            push_root(d, &mut st, root)
                        };
                        match result {
                            Ok(n) => {
                                files += n;
                                if let Some(clean_gen) =
                                    root_quiet_since(&d.watcher, root, event_gen)
                                {
                                    d.pre_synced.lock().unwrap().insert(
                                        root.clone(),
                                        PreSyncStamp {
                                            link_ptr: Arc::as_ptr(&link) as usize,
                                            at: Instant::now(),
                                            event_gen: clean_gen,
                                            files: n,
                                        },
                                    );
                                }
                            }
                            Err(e) => {
                                d.pre_synced.lock().unwrap().remove(root);
                                st.source_sync_failed(root);
                                log(&format!("presync {}: {e:#}", root.display()));
                                break;
                            }
                        }
                    }
                }
                write_frame(
                    &mut w,
                    &Msg::PreSyncDone {
                        files,
                        ms: t.elapsed().as_secs_f64() * 1000.0,
                    },
                    &[],
                )?;
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
            }
            | Msg::RunSync {
                cwd,
                args,
                env,
                roots,
                start,
                run_id,
                ..
            } => {
                d.set_info(|i| i.active += 1);
                let res = run(
                    d, &mut r, &mut w, cwd, args, env, roots, start, run_id, pipeline,
                );
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

fn warm_admit_body_ok(body: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .is_some_and(|v| {
            let update_active = !v["update"].is_null()
                && matches!(
                    v["update"]["state"].as_str(),
                    None | Some("announced" | "restarting")
                );
            v["admission"] == "maintenance-v1"
                && !update_active
                && crate::remote_hosted::Builds::from(&v["builds"]).is_some_and(|b| {
                    b.can_build
                        && (b.left() >= 2
                            || b.overage_price_usd.is_some()
                            || (b.trial_remaining.is_none() && b.included.is_none()))
                })
        })
}

fn record_warm_admit(d: &Daemon, key: &str, code: u16, body: &str) {
    let link_ptr = d.live_link_ptr.load(Ordering::SeqCst);
    let ok = code == 200 && link_ptr != 0 && warm_admit_body_ok(body);
    let mut slot = d.warm_admit.lock().unwrap();
    if ok {
        *slot = Some(WarmAdmit {
            key_hash: remote_checkpoint::digest(key.as_bytes()),
            link_ptr,
            at: Instant::now(),
        });
    } else {
        *slot = None;
    }
}

fn lease_eligible_body(body: &str) -> bool {
    if !warm_admit_body_ok(body) {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| crate::remote_hosted::Builds::from(&v["builds"]))
        .is_some_and(|b| {
            b.can_build
                && b.included
                    .is_some_and(|inc| inc.saturating_sub(b.included_used.unwrap_or(0)) >= 2)
        })
}

fn hosted_leases_enabled() -> bool {
    std::env::var("JUSTRUST_HOSTED_LEASES").as_deref() != Ok("0")
}

fn checkout_from_pool(
    pool: &mut LeasePool,
    key_hash: &str,
    live_link_ptr: usize,
    run_meta: serde_json::Value,
    now: Instant,
) -> Option<(String, bool)> {
    let lease = pool.active.as_mut()?;
    if !lease.usable_for(key_hash, live_link_ptr, now) {
        return None;
    }
    if lease.link_ptr == 0 {
        lease.link_ptr = live_link_ptr;
    }
    lease.used += 1;
    if let Some(used) = lease.builds["included_used"].as_u64() {
        lease.builds["included_used"] = serde_json::Value::from(used + 1);
    }
    lease.pending_runs.push(run_meta);
    let mut resp = serde_json::json!({
        "admission": "maintenance-v1",
        "update": serde_json::Value::Null,
        "builds": lease.builds.clone(),
        "lease_hit": true,
    });
    if let Some(fv) = lease.fleet_version.as_ref() {
        resp["fleet_version"] = serde_json::Value::String(fv.clone());
    }
    let need_refill = lease.needs_refill(now);
    let exhausted = lease.used >= lease.granted;
    if exhausted && let Some(done) = pool.active.take() {
        pool.draining.push(done);
    }
    Some((resp.to_string(), need_refill))
}

fn try_checkout_build_lease(d: &Arc<Daemon>, key: &str, body: &str) -> Option<String> {
    if !hosted_leases_enabled() || d.draining.load(Ordering::SeqCst) {
        return None;
    }
    let link_ptr = d.live_link_ptr.load(Ordering::SeqCst);
    if link_ptr == 0 {
        return None;
    }
    let run_meta = serde_json::from_str::<serde_json::Value>(body).ok()?;
    let has_run_id = run_meta
        .get("run_id")
        .and_then(|v| v.as_str())
        .is_some_and(|s| !s.is_empty());
    if !has_run_id {
        return None;
    }
    let key_hash = remote_checkpoint::digest(key.as_bytes());
    let now = Instant::now();
    let (resp_body, need_refill, spawn_redeem, spawn_refill) = {
        let mut pool = d.leases.lock().unwrap();
        let (resp_body, need_refill) =
            checkout_from_pool(&mut pool, &key_hash, link_ptr, run_meta, now)?;
        let spawn_redeem = if !pool.redeem_in_flight {
            pool.redeem_in_flight = true;
            true
        } else {
            false
        };
        let spawn_refill =
            if need_refill && !pool.refill_in_flight && pool.retry_after.is_none_or(|at| now >= at)
            {
                pool.refill_in_flight = true;
                true
            } else {
                false
            };
        (resp_body, need_refill, spawn_redeem, spawn_refill)
    };
    if let Ok(mut wa) = d.warm_admit.lock() {
        *wa = Some(WarmAdmit {
            key_hash,
            link_ptr,
            at: now,
        });
    }
    if spawn_redeem {
        spawn_redeem_leases(d.hosted_agent.clone(), d.leases.clone());
    }
    if need_refill && spawn_refill {
        spawn_refill_lease(d.clone(), key.to_owned(), link_ptr);
    }
    Some(resp_body)
}

fn maybe_warm_build_lease(d: &Arc<Daemon>, key: &str, body: &str) {
    if !hosted_leases_enabled() || d.draining.load(Ordering::SeqCst) || !lease_eligible_body(body) {
        return;
    }
    let link_ptr = d.live_link_ptr.load(Ordering::SeqCst);
    let key_hash = remote_checkpoint::digest(key.as_bytes());
    let now = Instant::now();
    let should_refill = {
        let mut pool = d.leases.lock().unwrap();
        let already_healthy = pool.active.as_ref().is_some_and(|l| {
            l.key_hash == key_hash
                && (l.link_ptr == 0 || link_ptr == 0 || l.link_ptr == link_ptr)
                && !l.needs_refill(now)
        });
        if already_healthy || pool.refill_in_flight || pool.retry_after.is_some_and(|at| now < at) {
            false
        } else {
            pool.refill_in_flight = true;
            true
        }
    };
    if should_refill {
        spawn_refill_lease(d.clone(), key.to_owned(), link_ptr);
    }
}

fn invalidate_active_lease(d: &Daemon) {
    let spawn_redeem = {
        let mut pool = d.leases.lock().unwrap();
        if let Some(old) = pool.active.take() {
            pool.draining.push(old);
        }
        if !pool.draining.is_empty() && !pool.redeem_in_flight {
            pool.redeem_in_flight = true;
            true
        } else {
            false
        }
    };
    if spawn_redeem {
        spawn_redeem_leases(d.hosted_agent.clone(), d.leases.clone());
    }
}

fn spawn_refill_lease(d: Arc<Daemon>, key: String, expected_link_ptr: usize) {
    std::thread::spawn(move || {
        let url = format!("{}/build/leases", crate::remote_hosted::DEFAULT_BUILD_BASE);
        let payload = serde_json::json!({
            "count": LEASE_BATCH_COUNT,
            "client": concat!("justrust ", env!("CARGO_PKG_VERSION")),
        })
        .to_string();
        let mut req = d
            .hosted_agent
            .post(&url)
            .header("Authorization", &format!("Bearer {key}"))
            .header("Content-Type", "application/json")
            .header("X-Justrust-Client-Version", crate::update::VERSION)
            .header("X-Justrust-Admission", "maintenance-v1");
        if let Some(target) = crate::update::target() {
            req = req.header("X-Justrust-Target", target);
        }
        let parsed = req.send(payload.as_bytes()).ok().and_then(|mut r| {
            let code = r.status().as_u16();
            let text = r
                .body_mut()
                .with_config()
                .limit(256 * 1024)
                .read_to_string()
                .ok()?;
            Some((code, text))
        });
        let mut trigger_redeem = false;
        let mut installed_lease = false;
        if let Some((200, text)) = parsed
            && let Ok(v) = serde_json::from_str::<serde_json::Value>(&text)
        {
            let update_active = !v["update"].is_null()
                && matches!(
                    v["update"]["state"].as_str(),
                    None | Some("announced" | "restarting")
                );
            let lease_id = v["lease"]["lease_id"].as_str().map(str::to_owned);
            let granted = v["lease"]["granted"].as_u64().unwrap_or(0);
            let ttl_secs = v["lease"]["ttl_secs"].as_u64().unwrap_or(0);
            let cur_link = d.live_link_ptr.load(Ordering::SeqCst);
            let link_ok = expected_link_ptr == 0 || cur_link == expected_link_ptr;
            if v["admission"] == "maintenance-v1"
                && !update_active
                && link_ok
                && granted > 0
                && ttl_secs > LEASE_SAFETY_MARGIN.as_secs()
                && let Some(lease_id) = lease_id
            {
                let fleet_version = v["fleet_version"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned);
                let new_lease = BuildLease {
                    lease_id,
                    key: key.clone(),
                    key_hash: remote_checkpoint::digest(key.as_bytes()),
                    link_ptr: cur_link,
                    granted,
                    used: 0,
                    expires_at: Instant::now() + Duration::from_secs(ttl_secs),
                    builds: v["builds"].clone(),
                    fleet_version,
                    pending_runs: Vec::new(),
                };
                let mut pool = d.leases.lock().unwrap();
                if let Some(old) = pool.active.replace(new_lease) {
                    pool.draining.push(old);
                }
                pool.retry_after = None;
                pool.refill_in_flight = false;
                if !pool.draining.is_empty() && !pool.redeem_in_flight {
                    pool.redeem_in_flight = true;
                    trigger_redeem = true;
                }
                installed_lease = true;
            }
        }
        if !installed_lease {
            let mut pool = d.leases.lock().unwrap();
            pool.retry_after = Some(Instant::now() + LEASE_RETRY_BACKOFF);
            pool.refill_in_flight = false;
        }
        if trigger_redeem {
            spawn_redeem_leases(d.hosted_agent.clone(), d.leases.clone());
        }
    });
}

fn send_redeem(
    agent: &ureq::Agent,
    key: &str,
    lease_id: &str,
    runs: &[serde_json::Value],
    close: bool,
) -> std::result::Result<u16, String> {
    let url = format!(
        "{}/build/leases/redeem",
        crate::remote_hosted::DEFAULT_BUILD_BASE
    );
    let payload = serde_json::json!({
        "lease_id": lease_id,
        "runs": runs,
        "close": close,
    })
    .to_string();
    let mut req = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {key}"))
        .header("Content-Type", "application/json")
        .header("X-Justrust-Client-Version", crate::update::VERSION);
    if let Some(target) = crate::update::target() {
        req = req.header("X-Justrust-Target", target);
    }
    match req.send(payload.as_bytes()) {
        Ok(mut r) => {
            let code = r.status().as_u16();
            let _ = r.body_mut().with_config().limit(64 * 1024).read_to_string();
            Ok(code)
        }
        Err(e) => Err(e.to_string()),
    }
}

fn replay_unredeemed_runs(agent: &ureq::Agent, key: &str, runs: &[serde_json::Value]) {
    let url = format!("{}/build/runs", crate::remote_hosted::DEFAULT_BUILD_BASE);
    for run in runs {
        let mut req = agent
            .post(&url)
            .header("Authorization", &format!("Bearer {key}"))
            .header("Content-Type", "application/json")
            .header("X-Justrust-Client-Version", crate::update::VERSION)
            .header("X-Justrust-Admission", "maintenance-v1");
        if let Some(target) = crate::update::target() {
            req = req.header("X-Justrust-Target", target);
        }
        if let Ok(mut r) = req.send(run.to_string().as_bytes()) {
            let _ = r.body_mut().with_config().limit(64 * 1024).read_to_string();
        }
    }
}

fn next_redeem_work(
    leases: &Mutex<LeasePool>,
    close_active: bool,
) -> Option<(String, String, Vec<serde_json::Value>, bool)> {
    let mut pool = leases.lock().unwrap();
    if let Some(draining) = pool.draining.pop() {
        return Some((draining.key, draining.lease_id, draining.pending_runs, true));
    }
    if close_active {
        if let Some(active) = pool.active.take() {
            return Some((active.key, active.lease_id, active.pending_runs, true));
        }
        return None;
    }
    if let Some(active) = pool.active.as_mut()
        && !active.pending_runs.is_empty()
    {
        let runs = std::mem::take(&mut active.pending_runs);
        return Some((active.key.clone(), active.lease_id.clone(), runs, false));
    }
    pool.redeem_in_flight = false;
    None
}

fn spawn_redeem_leases(agent: ureq::Agent, leases: Arc<Mutex<LeasePool>>) {
    std::thread::spawn(move || {
        while let Some((key, lease_id, runs, close)) = next_redeem_work(&leases, false) {
            match send_redeem(&agent, &key, &lease_id, &runs, close) {
                Ok(200) => {}
                Ok(404 | 409) => {
                    {
                        let mut pool = leases.lock().unwrap();
                        if pool.active.as_ref().is_some_and(|a| a.lease_id == lease_id) {
                            pool.active = None;
                        }
                    }
                    if !runs.is_empty() {
                        replay_unredeemed_runs(&agent, &key, &runs);
                    }
                }
                Ok(code) => {
                    log(&format!("build lease redeem {lease_id}: HTTP {code}"));
                }
                Err(e) => {
                    log(&format!("build lease redeem {lease_id}: {e}"));
                }
            }
        }
    });
}

fn flush_and_close_leases_on_exit(agent: &ureq::Agent, leases: &Arc<Mutex<LeasePool>>) {
    let mut pending = Vec::new();
    while let Some(item) = next_redeem_work(leases, true) {
        pending.push(item);
    }
    if pending.is_empty() {
        return;
    }
    let agent = agent.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for (key, lease_id, runs, close) in pending {
            if let Ok(404 | 409) = send_redeem(&agent, &key, &lease_id, &runs, close)
                && !runs.is_empty()
            {
                replay_unredeemed_runs(&agent, &key, &runs);
            }
        }
        let _ = tx.send(());
    });
    let _ = rx.recv_timeout(Duration::from_secs(3));
}

fn drop_link(d: &Daemon, st: &mut Sync) {
    let had_link = st.link.is_some() || d.live_link_ptr.load(Ordering::SeqCst) != 0;
    st.receipt = None;
    d.live_link_ptr.store(0, Ordering::SeqCst);
    if let Ok(mut wa) = d.warm_admit.lock() {
        *wa = None;
    }
    if had_link {
        invalidate_active_lease(d);
    }
    if let Some(l) = st.link.take() {
        l.peer.close();
        l.kill();
    }
    if let Ok(mut c) = d.peer.lock() {
        *c = None;
    }
    d.set_info(|i| i.connected = false);
}

/// Active mirror prefix on the connected remote machine (empty when paths
/// are mirrored at `/` without rebasing).
pub fn active_prefix() -> String {
    crate::remote::dir()
        .ok()
        .and_then(|d| std::fs::read_to_string(d.join("active-prefix")).ok())
        .map(|s| s.trim().to_string())
        .unwrap_or_default()
}

fn save_active_prefix(prefix: &str) {
    if let Ok(d) = crate::remote::dir() {
        let p = d.join("active-prefix");
        if prefix.is_empty() {
            let _ = std::fs::remove_file(p);
        } else {
            let _ = std::fs::write(p, prefix);
        }
    }
}

/// Each daemon owns its hosted source tree for its lifetime, including reconnects.
/// Checkpoints cannot exclude another writer after a build has started.
fn hosted_mirror_script(namespace: &str) -> String {
    format!(
        "mkdir -p ~/.justrust-mirror/{namespace} && cd ~/.justrust-mirror/{namespace} && pwd -P"
    )
}

fn hosted_mirror_isolation(value: Option<&str>) -> bool {
    matches!(value, Some("1" | "on" | "true" | "require"))
}

fn hosted_mirror_namespace() -> &'static str {
    static NAMESPACE: OnceLock<String> = OnceLock::new();
    NAMESPACE.get_or_init(remote_checkpoint::new_token)
}

const REMOTE_MIRROR_PREFIX_SCRIPT: &str =
    "mkdir -p ~/.justrust-mirror && cd ~/.justrust-mirror && pwd -P";

/// On user SSH machines (especially cross-OS Linux <-> macOS where `/home` is
/// controlled by `autofs` on macOS or `/Users` is not writable without root on
/// Linux), check whether the local home directory path can be prepared at `/`.
/// If not, rebase mirrors under the remote user's canonical `~/.justrust-mirror`.
fn resolve_remote_prefix(
    backend: &crate::remote_backend::Backend,
    target: &crate::remote_backend::Target,
) -> Result<String> {
    let script = if matches!(backend, crate::remote_backend::Backend::Hosted)
        && hosted_mirror_isolation(
            crate::remote_hosted::setting("JUSTRUST_HOSTED_MIRROR_ISOLATION").as_deref(),
        ) {
        hosted_mirror_script(hosted_mirror_namespace())
    } else if matches!(backend, crate::remote_backend::Backend::Ssh { .. }) {
        if let Some(home) = dirs::home_dir()
            && let Some(h) = home.to_str()
            && target
                .command()
                .arg(remote_sync::prepare_script(h))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        {
            return Ok(String::new());
        }
        REMOTE_MIRROR_PREFIX_SCRIPT.to_string()
    } else {
        return Ok(String::new());
    };
    let out = target
        .command()
        .arg(script)
        .stdin(Stdio::null())
        .output()
        .context("resolving remote mirror prefix")?;
    if !out.status.success() {
        bail!("cannot create ~/.justrust-mirror on the remote machine");
    }
    let prefix = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !prefix.starts_with('/') {
        bail!("remote mirror prefix is not an absolute path: {prefix:?}");
    }
    Ok(prefix)
}

/// How long a started agent may take to answer Hello. A healthy one answers
/// in under a second; session recovery gives up after three 10 s attempts.
const HELLO_TIMEOUT: Duration = Duration::from_secs(90);

/// Stop the process `pid` unless `greeted` is set within `timeout`.
/// Returns whether it gave up. `greeted` is set either way, so whoever
/// reaps the child can tell that the watchdog is finished with its pid.
fn stop_unless_greeted(pid: i32, greeted: &AtomicBool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if greeted.load(Ordering::SeqCst) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    if greeted.swap(true, Ordering::SeqCst) {
        return false;
    }
    #[cfg(unix)]
    // SAFETY: kill only signals. The child is not reaped before `greeted`
    // is set, so the pid is still ours.
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
    #[cfg(windows)]
    crate::sys::terminate(pid);
    true
}

/// Connect to the agent (starting the machine only when `start`) and begin
/// reading frames. Invalidate known roots: only roots needed by a build are
/// mirrored before that build, so an unrelated old workspace cannot delay it.
///
/// Takes the mirror lock and gives it back, but does not hold it while the
/// network is involved: on 2026-11-27 one connect took 427 s and eleven
/// builds queued on the lock behind it for up to 482 s. One caller connects,
/// the others wait for that attempt (no longer than a build may spend
/// connecting) and find its link, or get its failure as theirs. The link is
/// published under the lock.
fn ensure_link<'a>(
    d: &'a Daemon,
    mut st: std::sync::MutexGuard<'a, Sync>,
    start: bool,
    relock: Relock,
) -> (std::sync::MutexGuard<'a, Sync>, Result<Arc<Link>>) {
    let lock = || match relock {
        Relock::Foreground => d.sync.foreground_lock().unwrap(),
        Relock::Background => d.sync.background_lock().unwrap(),
    };
    loop {
        if let Some(l) = &st.link {
            if l.alive() {
                let l = l.clone();
                return (st, Ok(l));
            }
            log("connection to the agent was lost, reconnecting");
        }
        let lead = match d.sync.connect_turn(start) {
            sync_queue::Turn::Lead(lead) => lead,
            sync_queue::Turn::Follow(attempt) => {
                drop(st);
                let followed = attempt.follow(start, crate::remote_build::connect_bound());
                st = lock();
                match followed {
                    sync_queue::Followed::Connected | sync_queue::Followed::Retry => continue,
                    sync_queue::Followed::Failed(why) => {
                        return (
                            st,
                            Err(anyhow::anyhow!(
                                "remote connect failed while this build waited for it: {why}"
                            )),
                        );
                    }
                    sync_queue::Followed::Overdue(secs) => {
                        return (
                            st,
                            Err(anyhow::anyhow!(
                                "remote connect made for another build has not finished after {secs}s"
                            )),
                        );
                    }
                }
            }
        };
        drop_link(d, &mut st);
        if !start && !st.backend.probably_up() {
            let e = anyhow::anyhow!("remote machine is not running");
            lead.finish(Err(format!("{e:#}")));
            return (st, Err(e));
        }
        let backend = st.backend.clone();
        let mut retry = std::mem::take(&mut st.install_retry);
        drop(st);
        let opened = open_link(d, &backend, &mut retry);
        st = lock();
        st.install_retry = retry;
        let res = opened.and_then(|opened| {
            // The lock was free meanwhile: this link may no longer be wanted.
            if d.draining.load(Ordering::SeqCst) || st.backend != backend {
                opened.link.peer.close();
                opened.link.kill();
                bail!("remote machine or transport changed during the connect");
            }
            Ok(publish_link(d, &mut st, opened))
        });
        // Still under the lock: whoever follows finds the link in place.
        lead.finish(res.as_ref().map(|_| ()).map_err(|e| format!("{e:#}")));
        return (st, res);
    }
}

/// How a caller of [`ensure_link`] takes the mirror lock back.
#[derive(Clone, Copy)]
enum Relock {
    /// A build: ahead of background work.
    Foreground,
    /// Warmup or relink: after any build that is waiting.
    Background,
}

/// A connected agent that is not yet anyone's link.
struct Opened {
    link: Arc<Link>,
    dest: String,
    /// Machine ready, install, session.
    parts: [Duration; 3],
}

/// The network part of a connect. Holds no lock.
fn open_link(d: &Daemon, backend: &Backend, install_retry: &mut InstallRetry) -> Result<Opened> {
    let connecting = Instant::now();
    let target = backend.ensure_ready()?;
    let ready = connecting.elapsed();
    install_retry.run(&target.dest, Instant::now, || {
        crate::remote_build::ensure_remote_justrust(&target)
    })?;
    let prefix = resolve_remote_prefix(backend, &target)?;
    save_active_prefix(&prefix);
    let installed = connecting.elapsed();
    let mut command = if *backend == Backend::Hosted && crate::remote_session::enabled() {
        install_retry.run(&target.dest, Instant::now, || {
            crate::remote_session::command(&target, &prefix)
        })?
    } else {
        // This client's own copy on the machine when it has one.
        let agent_cmd = format!(
            "export PATH=\"$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.homebrew/bin:/usr/local/bin:$PATH\"; exec {} __agent{}",
            crate::remote_build::machine_justrust(),
            if prefix.is_empty() {
                String::new()
            } else {
                format!(" {}", crate::remote_build::shell_quote(&prefix))
            }
        );
        let mut command = target.command();
        command.arg(agent_cmd);
        command
    };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("starting the remote agent")?;
    // The agent reports write failures on stderr: drain it into our log so
    // a full pipe never stalls it.
    let outage = Arc::new(Outage::default());
    if let Some(mut e) = child.stderr.take() {
        let outage = outage.clone();
        std::thread::spawn(move || {
            let mut s = String::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = e.read(&mut buf) {
                if n == 0 {
                    break;
                }
                s.push_str(&String::from_utf8_lossy(&buf[..n]));
                while let Some(i) = s.find('\n') {
                    outage.observe(&s[..i]);
                    log(&format!("agent: {}", &s[..i]));
                    s.drain(..=i);
                }
            }
        });
    }
    let mut stdin = BufWriter::new(child.stdin.take().unwrap());
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    write_frame(
        &mut stdin,
        &Msg::Hello {
            version: VERSION,
            caps: Vec::new(),
        },
        &[],
    )?;
    // Nothing else bounds this read: every ssh step above ends within its
    // keepalives, but a session bridge that never hears back kept the sync
    // lock (and every build queued on it) for as long as it lived.
    let greeted = Arc::new(AtomicBool::new(false));
    {
        let (greeted, pid) = (greeted.clone(), child.id() as i32);
        std::thread::spawn(move || {
            if stop_unless_greeted(pid, &greeted, HELLO_TIMEOUT) {
                log(&format!(
                    "remote agent did not answer the handshake in {}s, gave up this connect",
                    HELLO_TIMEOUT.as_secs()
                ));
            }
        });
    }
    let hello = read_frame(&mut stdout);
    // From here the watchdog must never touch the pid.
    let timed_out = greeted.swap(true, Ordering::SeqCst) && !matches!(hello, Ok(Some(_)));
    let checkpoint = match hello {
        Ok(Some((Msg::Hello { version, caps }, _))) if version == VERSION => {
            remote_checkpoint::enabled() && caps.iter().any(|c| c == remote_checkpoint::CAP)
        }
        other => {
            let _ = child.kill();
            let _ = child.wait();
            let answered = match &other {
                Ok(Some((Msg::Hello { version, .. }, _))) => Some(*version),
                _ => None,
            };
            // An agent that never answered says nothing about its binary.
            if !timed_out
                && stamp_is_stale(answered, || {
                    crate::remote_build::remote_justrust_intact(&target)
                })
                && let Ok(d) = crate::remote::dir()
            {
                let _ = std::fs::remove_file(d.join("remote-justrust"));
            }
            if timed_out {
                bail!(
                    "remote agent handshake timed out after {}s",
                    HELLO_TIMEOUT.as_secs()
                );
            }
            bail!("remote agent handshake failed: {other:?}");
        }
    };
    let stdin = Arc::new(Mutex::new(stdin));
    let peer = {
        let w = stdin.clone();
        peercache::Relay::new(true, move |m, p| {
            w.lock()
                .is_ok_and(|mut w| write_frame(&mut *w, m, p).is_ok())
        })
    };
    let link = Arc::new(Link {
        child: Mutex::new(child),
        stdin,
        outage,
        peer: peer.clone(),
        checkpoint,
        routes: Mutex::default(),
        mirror: Mirror {
            target: Some(target.clone()),
            prefix,
        },
    });
    {
        let link = link.clone();
        let hint = d.status_hint.clone();
        let relink = d.relink.clone();
        std::thread::spawn(move || {
            read_agent(&link, stdout, hint);
            // The machine closing an idle session is not a reason to come
            // back. A transport that dropped under a live session is.
            if link.outage.is_down()
                && let Some(tx) = relink
            {
                let _ = tx.try_send(());
            }
        });
    }
    Ok(Opened {
        link,
        dest: target.dest.clone(),
        parts: [
            ready,
            installed.saturating_sub(ready),
            connecting.elapsed().saturating_sub(installed),
        ],
    })
}

/// Make an opened agent the daemon's link. Under the mirror lock.
fn publish_link(d: &Daemon, st: &mut Sync, opened: Opened) -> Arc<Link> {
    let Opened { link, dest, parts } = opened;
    st.link_parts = Some(parts);
    log(&format!(
        "connected to {dest} in {:.2}s: ready {:.2}s, install {:.2}s, session {:.2}s",
        parts.iter().sum::<Duration>().as_secs_f64(),
        parts[0].as_secs_f64(),
        parts[1].as_secs_f64(),
        parts[2].as_secs_f64()
    ));
    let peer = link.peer.clone();
    peer.prefetch_keys();
    if let Ok(mut c) = d.peer.lock() {
        *c = Some(peer);
    }
    d.live_link_ptr
        .store(Arc::as_ptr(&link) as usize, Ordering::SeqCst);
    st.link = Some(link.clone());
    d.set_info(|i| i.connected = true);
    // Everything the machine had may be stale: start from a full mirror,
    // unless the agent confirms a root's checkpoint (`reverify`).
    // Roots deleted since (a removed worktree) are forgotten, not mirrored:
    // rsync of a missing dir failed the whole connect and sent the next
    // build local (FINDINGS 65).
    st.invalidate_sources();
    update_root_info(d, st);
    link
}

fn read_agent(
    link: &Link,
    mut stdout: BufReader<std::process::ChildStdout>,
    status_hint: Option<std::sync::mpsc::SyncSender<()>>,
) {
    let disconnect_kind = loop {
        let (msg, payload) = match read_frame(&mut stdout) {
            Ok(Some(frame)) => frame,
            Ok(None) => break "connection_closed",
            Err(error) => {
                let kind = crate::remote_build::transport_error_kind(&error);
                break if kind == "unknown" { "protocol" } else { kind };
            }
        };
        if link.peer.on_frame(&msg, &payload) {
            continue;
        }
        let mut r = link.routes.lock().unwrap();
        match msg {
            Msg::BarrierAck {
                id,
                failed,
                disk_full,
                lost,
            } => {
                if let Some(tx) = r.barriers.remove(&id) {
                    let _ = tx.send(SyncAck {
                        failed,
                        disk_full,
                        lost,
                    });
                }
            }
            Msg::Verified { id, ok, why } => {
                if let Some(tx) = r.verifies.remove(&id) {
                    let _ = tx.send(if ok { Ok(()) } else { Err(why) });
                }
            }
            Msg::Out { id, .. }
            | Msg::RunFiles { id }
            | Msg::Exit { id, .. }
            | Msg::Error { id, .. }
                if id != 0 =>
            {
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
    };
    // Connection lost: fail every waiter so clients fall back or stop.
    link.peer.close();
    let mut r = link.routes.lock().unwrap();
    r.dead = true;
    // Ping derives connectivity from Link::alive even before Info is updated.
    // An old draining link may wake a replacement's watcher, which simply
    // recollects the current socket's state rather than trusting this marker.
    if let Some(tx) = status_hint {
        let _ = tx.try_send(());
    }
    r.barriers.clear();
    r.verifies.clear();
    for (id, tx) in r.runs.drain() {
        let _ = tx.send((
            Msg::Error {
                id,
                msg: format!("connection to the remote machine was lost: {disconnect_kind}"),
            },
            Vec::new(),
        ));
    }
    log(&format!("agent connection closed: {disconnect_kind}"));
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
fn refresh_watches(d: &Daemon, root: &Path, dirs: &std::collections::BTreeSet<PathBuf>) -> usize {
    let n = d.watcher.watch_all(dirs);
    if n > 0 {
        log(&format!("watching {n} more dirs under {}", root.display()));
    }
    n
}

/// Roots this client ever mirrored to `dest` (`mirrored.json`), so a new
/// worktree can be seeded from one even after the daemon restarted.
fn known_roots(dest: &str) -> Vec<PathBuf> {
    let dir = crate::remote::dir().ok();
    // Roots mirrored before `mirrored.json` existed ("instance:/path").
    let prepared: Vec<PathBuf> = dir
        .as_ref()
        .and_then(|d| std::fs::read(d.join("prepared.json")).ok())
        .and_then(|b| serde_json::from_slice::<Vec<String>>(&b).ok())
        .unwrap_or_default()
        .iter()
        .filter_map(|s| s.split_once(':').map(|(_, p)| PathBuf::from(p)))
        .collect();
    dir.and_then(|d| std::fs::read(d.join("mirrored.json")).ok())
        .and_then(|b| serde_json::from_slice::<BTreeMap<String, Vec<PathBuf>>>(&b).ok())
        .and_then(|mut m| m.remove(dest))
        .unwrap_or_default()
        .into_iter()
        .chain(prepared)
        .filter(|p| p.is_dir())
        .collect()
}

fn remember_root(dest: &str, root: &Path) {
    let Ok(d) = crate::remote::dir() else { return };
    let path = d.join("mirrored.json");
    let mut m: BTreeMap<String, Vec<PathBuf>> = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let list = m.entry(dest.to_string()).or_default();
    if list.iter().any(|p| p == root) {
        return;
    }
    list.push(root.to_path_buf());
    // Keep the newest 64: candidates are probed with git on each new root.
    let excess = list.len().saturating_sub(64);
    list.drain(..excess);
    if let Ok(s) = serde_json::to_vec(&m) {
        let _ = std::fs::write(&path, s);
    }
}

/// Save what the machine holds, for the next daemon (`remote_resume`). Only
/// after a barrier confirms every frame applied, and only roots with no
/// pending repair or full sync.
fn save_mirrors(d: &Daemon) {
    if !remote_resume::enabled() {
        return;
    }
    // Never wait behind a sync in progress: a replacement daemon is waiting
    // for us to let go, and that sync's state is not settled anyway.
    let Ok(mut st) = d.sync.try_lock() else {
        log("mirror manifests not saved: a sync is in progress");
        return;
    };
    let Some(link) = st.link.clone().filter(|l| l.alive()) else {
        log("mirror manifests not saved: no live connection");
        return;
    };
    if let Err(e) = barrier(d, &mut st, &link, None) {
        st.receipt = None;
        log(&format!("mirror manifests not saved: {e:#}"));
        return;
    }
    let clean: Vec<(&Path, &Manifest, &[PathBuf])> = st
        .roots
        .iter()
        .filter(|(r, root)| !root.needs_full_sync && r.is_dir() && !root.pending_repair)
        .map(|(r, root)| (r.as_path(), &root.manifest, root.oversized.as_slice()))
        .collect();
    let t = Instant::now();
    let n = remote_resume::save(st.backend.kind(), &link.mirror, &clean);
    log(&format!(
        "saved {n} of {} mirror manifests for the next daemon in {:.2}s",
        clean.len(),
        t.elapsed().as_secs_f64()
    ));
}

/// A root this daemon does not track yet: resume the previous daemon's
/// mirror when the machine confirms it, else mirror it in full. Returns
/// files sent.
fn mirror_root(d: &Daemon, st: &mut Sync, link: &Link, root: &Path) -> Result<usize> {
    st.receipt = None;
    if !st.roots.contains_key(root) {
        let t = Instant::now();
        match remote_resume::take(st.backend.kind(), &link.mirror, root) {
            Ok((manifest, oversized)) => {
                st.roots.insert(
                    root.to_path_buf(),
                    Root {
                        pending_repair: manifest.values().any(|s| *s == UNKNOWN),
                        manifest,
                        watch_dirs: None,
                        oversized,
                        needs_full_sync: false,
                        mark: Default::default(),
                        clean_event_gen: None,
                    },
                );
                remember_root(st.backend.kind(), root);
                let n = push_root(d, st, root)?;
                log(&format!(
                    "source mirror {}: resumed from the previous daemon, {n} files sent, total {:.3}s",
                    root.display(),
                    t.elapsed().as_secs_f64()
                ));
                return Ok(n);
            }
            Err(why) if why == "nothing saved" => {}
            Err(why) => log(&format!(
                "source mirror {}: not resumed ({why}, {:.3}s), full mirror",
                root.display(),
                t.elapsed().as_secs_f64()
            )),
        }
    } else if let Some(sent) = reverify(d, st, link, root) {
        return sent;
    }
    remote_resume::invalidate(&link.mirror, root);
    add_root(d, st, link, root).map(|()| 0)
}

/// A root this daemon mirrored over an earlier connection: keep its mirror
/// when the agent confirms the checkpoint (`remote_checkpoint`), then send
/// what the old connection may not have applied and what changed since.
/// `None` means no confirmation: mirror it in full.
fn reverify(d: &Daemon, st: &mut Sync, link: &Link, root: &Path) -> Option<Result<usize>> {
    if !link.checkpoint {
        return None;
    }
    let t = Instant::now();
    let (token, list, files) = {
        let r = st.roots.get(root)?;
        let token = r.mark.token.clone()?;
        let settled = || {
            r.manifest
                .iter()
                .filter(|(p, s)| **s != UNKNOWN && !r.mark.dirty.contains(*p))
        };
        let entries: Option<Vec<_>> = settled()
            .map(|(p, s)| Some((p.as_path(), s.size, r.mark.hashes.get(p)?.as_str())))
            .collect();
        let list = remote_checkpoint::encode(entries?.into_iter())?;
        (token, list, settled().count())
    };
    let next = remote_checkpoint::new_token();
    let id = d.id();
    let (tx, rx) = channel();
    {
        let mut routes = link.routes.lock().unwrap();
        if routes.dead {
            return None;
        }
        routes.verifies.insert(id, tx);
    }
    let answer = link
        .send(
            &Msg::Verify {
                id,
                path: root.display().to_string(),
                token,
                next: next.clone(),
            },
            &list,
        )
        .map_err(|e| e.to_string())
        .and_then(|()| {
            rx.recv_timeout(BARRIER_TIMEOUT)
                .map_err(|_| "no answer from the machine".to_string())?
        });
    link.routes.lock().unwrap().verifies.remove(&id);
    let r = st.roots.get_mut(root)?;
    if let Err(why) = answer {
        log(&format!(
            "source mirror {}: checkpoint not confirmed ({why}, {:.3}s), full mirror",
            root.display(),
            t.elapsed().as_secs_f64()
        ));
        r.mark = Default::default();
        return None;
    }
    let verified = t.elapsed();
    r.mark.token = Some(next);
    // Sent after the checkpoint: the old connection may or may not have
    // applied them. Absent locally means delete, as for a failed delete.
    let resend = r.mark.dirty.len();
    r.pending_repair |= !r.mark.dirty.is_empty();
    for path in &r.mark.dirty {
        r.manifest.insert(path.clone(), UNKNOWN);
    }
    r.needs_full_sync = false;
    Some(push_root(d, st, root).inspect(|n| {
        log(&format!(
            "source mirror {}: checkpoint confirmed for {files} files in {:.3}s, {resend} resent, {n} files sent, total {:.3}s",
            root.display(),
            verified.as_secs_f64(),
            t.elapsed().as_secs_f64()
        ));
    }))
}

/// The root is in sync once the agent has applied every frame so far: have
/// it remember that. Sent without waiting. An agent that failed one of those
/// frames refuses, and the next connection then mirrors in full.
fn checkpoint(st: &mut Sync, link: &Link, root: &Path, force: bool) -> Result<()> {
    if !link.checkpoint {
        return Ok(());
    }
    let Some(r) = st.roots.get_mut(root) else {
        return Ok(());
    };
    if !force && !r.mark.wanted() {
        return Ok(());
    }
    let token = remote_checkpoint::new_token();
    // Forget the old one first: if this frame is lost, nothing is claimed.
    r.mark.token = None;
    r.mark.dirty.clear();
    link.send(
        &Msg::Checkpoint {
            path: root.display().to_string(),
            token: token.clone(),
        },
        &[],
    )?;
    r.mark.token = Some(token);
    Ok(())
}

fn prepare_observing<T>(
    mirror: &Mirror,
    root: &Path,
    observe: impl FnOnce() -> Result<T>,
) -> Result<(T, Duration, Duration)> {
    std::thread::scope(|scope| {
        let prepare = || {
            let start = Instant::now();
            mirror.prepare(root).map(|()| start.elapsed())
        };
        let job = std::thread::Builder::new().spawn_scoped(scope, prepare);
        let start = Instant::now();
        let observed = observe();
        let scanned = start.elapsed();
        let prepared = match job {
            Ok(job) => job
                .join()
                .map_err(|_| anyhow::anyhow!("mirror preparation panicked"))?,
            Err(_) => prepare(),
        }?;
        Ok((observed?, prepared, scanned))
    })
}

/// Start tracking a root: rsync it once, then remember its manifest.
fn add_root(d: &Daemon, st: &mut Sync, link: &Link, root: &Path) -> Result<()> {
    let total = Instant::now();
    // Preparation touches only the receiver. Observe the sender concurrently,
    // but join both operations before seeding or copying any sources.
    let ((scan, hashes), prepared, scanned) = prepare_observing(&link.mirror, root, || {
        remote_sync::scan(root).map(|scan| {
            let hashes = if link.checkpoint {
                snapshot_hashes(root, &scan.files)
            } else {
                Default::default()
            };
            (scan, hashes)
        })
    })?;
    let t = Instant::now();
    // Keyed by backend, not address: hosted machines change addresses, and
    // a seed whose source is gone on the machine is simply skipped.
    // Put currently active roots on this host first so ties prefer warm mirrors.
    let dest = st.backend.kind();
    let mut known: Vec<PathBuf> = st.roots.keys().cloned().collect();
    known.extend(known_roots(dest));
    let mut seeded = false;
    // `JUSTRUST_REMOTE_SEED=0` (in the daemon's environment) turns off both
    // seeds, for measuring and as an escape hatch: a full rsync instead.
    let seeding = std::env::var("JUSTRUST_REMOTE_SEED").as_deref() != Ok("0");
    if seeding {
        for from in remote_sync::pick_seeds(root, known.iter())
            .into_iter()
            .take(3)
        {
            if link.mirror.seed(root, &from) {
                seeded = true;
                log(&format!(
                    "seeded {} from {} on the machine in {:.2}s",
                    root.display(),
                    from.display(),
                    t.elapsed().as_secs_f64()
                ));
                break;
            }
        }
    }
    if seeding
        && !seeded
        && std::env::var("JUSTRUST_REMOTE_GIT_SEED").as_deref() != Ok("0")
        && link.mirror.seed_from_origin(root)
    {
        seeded = true;
        log(&format!(
            "seeded {} from its git origin on the machine in {:.2}s",
            root.display(),
            t.elapsed().as_secs_f64()
        ));
    }
    let seed_time = t.elapsed();
    let transfer = Instant::now();
    let n = link.mirror.rsync_with(root, seeded)?;
    let transfer_time = transfer.elapsed();
    remember_root(dest, root);
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
    let watch_dirs = remote_sync::dirs(root, &scan.files);
    refresh_watches(d, root, &watch_dirs);
    log(&format!(
        "source mirror {}: total {:.3}s, prepare {:.3}s overlapping scan {:.3}s ({} files, {} oversized), seed {:.3}s, checksum rsync {:.3}s, finalize {:.3}s",
        root.display(),
        total.elapsed().as_secs_f64(),
        prepared.as_secs_f64(),
        scanned.as_secs_f64(),
        scan.files.len(),
        scan.oversized.len(),
        seed_time.as_secs_f64(),
        transfer_time.as_secs_f64(),
        transfer
            .elapsed()
            .saturating_sub(transfer_time)
            .as_secs_f64(),
    ));
    st.roots.insert(
        root.to_path_buf(),
        Root {
            pending_repair: false,
            manifest: scan.files,
            watch_dirs: Some(watch_dirs),
            oversized: scan.oversized,
            needs_full_sync: false,
            mark: remote_checkpoint::Mark {
                hashes,
                ..Default::default()
            },
            clean_event_gen: None,
        },
    );
    checkpoint(st, link, root, true)?;
    update_root_info(d, st);
    Ok(())
}

/// The content of `p`, only if it is exactly the file the scan stamped: the
/// stamp recorded in the manifest must describe the bytes sent. A file
/// rewritten between scan and read (truncated, then refilled, with its old
/// mtime restored as `cp -p` and `shutil.copy2` do) would otherwise be sent
/// half-written under a stamp that later matches again, and never resent.
fn read_settled(p: &Path, scanned: Option<remote_sync::Stamp>) -> Option<Vec<u8>> {
    let scanned = scanned?;
    let data = std::fs::read(p).ok()?;
    let after = std::fs::symlink_metadata(p).ok()?;
    let (s, ns) = crate::sys::mtime(&after);
    let now = s as i128 * 1_000_000_000 + ns as i128;
    (data.len() as u64 == scanned.size && after.len() == scanned.size && now == scanned.mtime_ns)
        .then_some(data)
}

fn snapshot_hashes(
    root: &Path,
    manifest: &Manifest,
) -> std::collections::BTreeMap<PathBuf, String> {
    // Hash on every core: serially this reads and hashes ~1.4 GB for a
    // large root and delays the first mirror by seconds.
    let entries: Vec<_> = manifest.iter().collect();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .clamp(1, 16);
    let chunk = entries.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let jobs: Vec<_> = entries
            .chunks(chunk)
            .map(|part| {
                scope.spawn(move || {
                    part.iter()
                        .filter_map(|(path, stamp)| {
                            let bytes = read_settled(&root.join(path), Some(**stamp))?;
                            Some(((*path).clone(), remote_checkpoint::digest(&bytes)))
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        jobs.into_iter()
            .flat_map(|j| j.join().unwrap_or_default())
            .collect()
    })
}

fn root_quiet_since(watcher: &Watcher, root: &Path, event_gen: u64) -> Option<u64> {
    if watcher.is_quiet_since(event_gen) {
        return Some(event_gen);
    }
    let (new_gen, dirty) = watcher.dirty_since(root, event_gen)?;
    dirty.is_empty().then_some(new_gen)
}

#[derive(Debug, Default, PartialEq, Eq)]
struct DirtyPlan {
    changed: Vec<(PathBuf, remote_sync::Stamp)>,
    deleted: Vec<PathBuf>,
}

/// Validate and classify exact watcher-reported paths against `root`'s manifest.
/// Returns `None` whenever a full `remote_sync::scan` is required (new persistent
/// files whose gitignore status must be evaluated, `.gitignore`/`.ignore` edits,
/// symlinks/directories, oversized files, or more than `RSYNC_ABOVE` paths).
fn plan_dirty_paths(root: &Path, tracked: &Root, dirty_paths: &[PathBuf]) -> Option<DirtyPlan> {
    if dirty_paths.len() > remote_sync::RSYNC_ABOVE {
        return None;
    }
    let mut plan = DirtyPlan::default();
    let mut ignore = None;
    for full in dirty_paths {
        let rel = full.strip_prefix(root).ok()?;
        if rel.as_os_str().is_empty() {
            return None;
        }
        if rel.file_name().is_some_and(|n| {
            n == ".gitignore"
                || n == ".ignore"
                || n == ".gitattributes"
                || n == ".gitmodules"
                || n == ".justrustignore"
        }) {
            return None;
        }
        // A path this root's `.justrustignore` excludes is not this root's:
        // its mirror never holds it, and a nested root (a workspace inside
        // the excluded tree, `remote_build::excluded_root`) reports its own
        // edits here too. Without this every such edit cost a full scan.
        if !tracked.manifest.contains_key(rel)
            && ignore
                .get_or_insert_with(|| remote_sync::JustrustIgnore::load(root))
                .matches(rel)
        {
            continue;
        }
        if tracked.oversized.iter().any(|p| p == rel) {
            return None;
        }
        match std::fs::symlink_metadata(full) {
            Ok(md) => {
                if !md.file_type().is_file() || md.len() > MAX_FILE {
                    return None;
                }
                let &old = tracked.manifest.get(rel)?;
                let (s, ns) = crate::sys::mtime(&md);
                let mode = crate::sys::mode(&md, full) & 0o777;
                let stamp = remote_sync::Stamp {
                    mtime_ns: s as i128 * 1_000_000_000 + ns as i128,
                    size: md.len(),
                    mode,
                };
                if stamp != old {
                    plan.changed.push((rel.to_path_buf(), stamp));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                if tracked.manifest.contains_key(rel) {
                    plan.deleted.push(rel.to_path_buf());
                }
            }
            Err(_) => return None,
        }
    }
    Some(plan)
}

fn push_dirty_paths(
    d: &Daemon,
    st: &mut Sync,
    link: &Arc<Link>,
    root: &Path,
    dirty_paths: &[PathBuf],
    new_gen: u64,
) -> Result<Option<usize>> {
    let plan = {
        let r = st.roots.get(root).context("unknown root")?;
        let Some(plan) = plan_dirty_paths(root, r, dirty_paths) else {
            return Ok(None);
        };
        plan
    };
    let link_ptr = Arc::as_ptr(link) as usize;
    if plan.changed.is_empty() && plan.deleted.is_empty() {
        if let Some(r) = st.roots.get_mut(root) {
            r.clean_event_gen = Some((link_ptr, new_gen));
        }
        return Ok(Some(0));
    }
    st.receipt = None;
    if link.checkpoint
        && let Some(r) = st.roots.get_mut(root)
    {
        r.mark
            .dirty
            .extend(plan.changed.iter().map(|(rel, _)| rel.clone()));
        r.mark.dirty.extend(plan.deleted.iter().cloned());
    }
    let mut pending_repair = false;
    for (rel, stamp) in &plan.changed {
        let p = root.join(rel);
        match read_settled(&p, Some(*stamp)) {
            Some(data) if data.len() as u64 <= MAX_FILE => {
                link.send(
                    &Msg::Write {
                        path: p.display().to_string(),
                        mode: stamp.mode,
                    },
                    &data,
                )?;
                let r = st.roots.get_mut(root).unwrap();
                r.manifest.insert(rel.clone(), *stamp);
                if link.checkpoint {
                    r.mark
                        .hashes
                        .insert(rel.clone(), remote_checkpoint::digest(&data));
                }
            }
            _ => {
                pending_repair = true;
                st.roots
                    .get_mut(root)
                    .unwrap()
                    .manifest
                    .insert(rel.clone(), UNKNOWN);
            }
        }
    }
    for rel in &plan.deleted {
        let r = st.roots.get_mut(root).unwrap();
        r.manifest.remove(rel);
        r.mark.hashes.remove(rel);
        link.send(
            &Msg::Delete {
                path: root.join(rel).display().to_string(),
            },
            &[],
        )?;
    }
    if let Some(r) = st.roots.get_mut(root) {
        r.pending_repair = pending_repair;
    }
    checkpoint(st, link, root, false)?;
    if let Some(r) = st.roots.get_mut(root) {
        r.clean_event_gen = if !pending_repair {
            Some((link_ptr, new_gen))
        } else {
            None
        };
    }
    let n = plan.changed.len() + plan.deleted.len();
    update_root_info(d, st);
    d.set_info(|i| i.pushed += n as u64);
    Ok(Some(n))
}

/// Push every difference between `root` and its manifest. Returns files sent.
fn push_root(d: &Daemon, st: &mut Sync, root: &Path) -> Result<usize> {
    if let Some(link) = st.link.clone()
        && let Some((link_ptr, event_gen)) = st
            .roots
            .get(root)
            .filter(|r| !r.pending_repair && !r.needs_full_sync)
            .and_then(|r| r.clean_event_gen)
        && link_ptr == Arc::as_ptr(&link) as usize
    {
        if d.watcher.is_quiet_since(event_gen) {
            return Ok(0);
        }
        if let Some((new_gen, dirty_paths)) = d.watcher.dirty_since(root, event_gen) {
            if dirty_paths.is_empty() {
                if let Some(r) = st.roots.get_mut(root) {
                    r.clean_event_gen = Some((link_ptr, new_gen));
                }
                return Ok(0);
            }
            if let Some(n) = push_dirty_paths(d, st, &link, root, &dirty_paths, new_gen)? {
                return Ok(n);
            }
        }
    }
    let event_gen = d.watcher.event_count();
    let total = Instant::now();
    let scan = remote_sync::scan(root)?;
    push_scanned(d, st, root, scan, total, total.elapsed(), event_gen)
}

fn push_scanned(
    d: &Daemon,
    st: &mut Sync,
    root: &Path,
    scan: remote_sync::Scan,
    total: Instant,
    scanned: Duration,
    event_gen: u64,
) -> Result<usize> {
    let diff_started = Instant::now();
    let mut new = scan.files;
    let mut pending_repair = false;
    let diff = {
        let r = st.roots.get(root).context("unknown root")?;
        remote_sync::diff(&r.manifest, &new)
    };
    if !diff.is_empty()
        || diff.spelling_changed
        || !st.roots[root]
            .oversized
            .iter()
            .map(|p| p.as_os_str())
            .eq(scan.oversized.iter().map(|p| p.as_os_str()))
    {
        st.receipt = None;
    }
    let comparison = diff_started.elapsed();
    let compared = total.elapsed();
    let link = st.link.clone().context("not connected")?;
    let mut bulk = false;
    if !diff.is_empty() {
        // Before anything is sent: after a lost connection these are the
        // paths whose state on the machine is unknown.
        if link.checkpoint
            && let Some(r) = st.roots.get_mut(root)
        {
            r.mark.dirty.extend(diff.changed.iter().cloned());
            r.mark.dirty.extend(diff.deleted.iter().cloned());
        }
        if diff.changed.len() + diff.deleted.len() > remote_sync::RSYNC_ABOVE {
            // A branch switch or a big generated change: one rsync is cheaper.
            // It bypasses the agent, so drop saved mirror state here.
            remote_resume::invalidate(&link.mirror, root);
            if link.checkpoint {
                st.roots.get_mut(root).unwrap().mark.hashes = snapshot_hashes(root, &new);
            }
            let t = Instant::now();
            let n = link.mirror.rsync(root)?;
            bulk = true;
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
                let scanned = new.get(rel).copied();
                match read_settled(&p, scanned) {
                    Some(data) if data.len() as u64 <= MAX_FILE => {
                        let mode = scanned.map_or(0o644, |s| s.mode);
                        link.send(
                            &Msg::Write {
                                path: p.display().to_string(),
                                mode,
                            },
                            &data,
                        )?;
                        if link.checkpoint {
                            st.roots
                                .get_mut(root)
                                .unwrap()
                                .mark
                                .hashes
                                .insert(rel.clone(), remote_checkpoint::digest(&data));
                        }
                    }
                    // Gone, grown, or rewritten since the scan: try again
                    // next time.
                    _ => {
                        pending_repair = true;
                        new.insert(rel.clone(), UNKNOWN);
                    }
                }
            }
            for rel in &diff.deleted {
                st.roots.get_mut(root).unwrap().mark.hashes.remove(rel);
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
    let sent = total.elapsed();
    let tracked = st.roots.get_mut(root).context("unknown root")?;
    let dirs = tracked.watch_directories(root, &new, &diff);
    let added_watches = refresh_watches(d, root, dirs);
    if total.elapsed() >= Duration::from_secs(1) {
        log(&format!(
            "source push {}: total {:.3}s, scan {:.3}s ({} files, {} oversized), diff {:.3}s, transfer {:.3}s ({} changed, {} deleted), watches {:.3}s",
            root.display(),
            total.elapsed().as_secs_f64(),
            scanned.as_secs_f64(),
            new.len(),
            scan.oversized.len(),
            comparison.as_secs_f64(),
            (sent - compared).as_secs_f64(),
            diff.changed.len(),
            diff.deleted.len(),
            (total.elapsed() - sent).as_secs_f64(),
        ));
    }
    if let Some(r) = st.roots.get_mut(root) {
        r.pending_repair = pending_repair;
        r.manifest = new;
        r.oversized = scan.oversized;
    }
    // `Synced` after a bulk rsync dropped the agent's checkpoint.
    checkpoint(st, &link, root, bulk)?;
    if let Some(r) = st.roots.get_mut(root) {
        r.clean_event_gen = if !pending_repair && added_watches == 0 {
            root_quiet_since(&d.watcher, root, event_gen)
                .map(|clean_gen| (Arc::as_ptr(&link) as usize, clean_gen))
        } else {
            None
        };
    }
    update_root_info(d, st);
    d.set_info(|i| i.pushed += n as u64);
    Ok(n)
}

/// Retry rejected source operations using the current local source, never by
/// clearing the agent's failure receipt. Its next barrier remains authoritative.
/// In particular an unavailable workspace is not an empty tree to delete.
fn repair_sources(d: &Daemon, st: &mut Sync, link: &Link) -> Result<usize> {
    let mut files = 0;
    for root in st.repair_roots() {
        log(&format!(
            "retrying rejected source operations under {}",
            root.display()
        ));
        std::fs::read_dir(&root)
            .with_context(|| format!("cannot repair failed source sync for {}", root.display()))?;
        for (rel, stamp) in &st.roots[&root].manifest {
            if *stamp != UNKNOWN {
                continue;
            }
            let path = root.join(rel);
            match std::fs::symlink_metadata(&path) {
                Ok(md) if md.is_file() => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Ok(_) => bail!("cannot repair changed source type at {}", path.display()),
                Err(e) => {
                    return Err(e).with_context(|| {
                        format!("cannot inspect failed source {}", path.display())
                    });
                }
            }
        }
        if st.needs_mirror(&root) {
            files += mirror_root(d, st, link, &root)?;
        } else {
            files += push_root(d, st, &root)?;
        }
        if st.roots[&root].pending_repair {
            bail!(
                "cannot read source files needed to repair {}",
                root.display()
            );
        }
    }
    Ok(files)
}

/// Wait until the agent has applied everything sent so far. Paths the agent
/// failed to write or delete fail the barrier, and are marked so the next
/// push sends them again.
fn barrier(d: &Daemon, st: &mut Sync, link: &Link, run_roots: Option<&[PathBuf]>) -> Result<()> {
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
    let ack = rx
        .recv_timeout(BARRIER_TIMEOUT)
        .map_err(|_| anyhow::anyhow!("remote machine did not acknowledge the sync"))?;
    check_sync_failures(st, &ack, run_roots)
}

/// The agent's answer to a barrier: paths it could not apply, and whether
/// any of them failed because the machine's disk is full.
#[derive(Debug, Default)]
struct SyncAck {
    failed: Vec<String>,
    disk_full: bool,
    /// Roots another connection wrote to since this one's checkpoint.
    lost: Vec<String>,
}

/// Both standalone and run-integrated barriers repair the same manifest.
/// `run_roots` are the roots a build reads. A root another connection
/// displaced is mirrored in full next time either way, but it rejects only
/// builds that read it (`None`: every root counts). An unrelated workspace,
/// for example a checkout deleted here after its build, must not fail the
/// next build of a different one.
fn check_sync_failures(st: &mut Sync, ack: &SyncAck, run_roots: Option<&[PathBuf]>) -> Result<()> {
    let failed = &ack.failed;
    if !failed.is_empty() || !ack.lost.is_empty() {
        st.receipt = None;
    }
    for root in &ack.lost {
        // Not what this daemon sent any more, and not known how: mirror it
        // again before anything builds on it.
        if let Some(r) = st.roots.get_mut(Path::new(root)) {
            r.needs_full_sync = true;
            r.mark = Default::default();
        }
        log(&format!(
            "source mirror {root}: changed by another connection, full mirror next"
        ));
    }
    let blocking: Vec<&str> = match run_roots {
        Some(roots) => remote_checkpoint::blocking(&ack.lost, roots),
        None => ack.lost.iter().map(String::as_str).collect(),
    };
    if failed.is_empty() {
        if !blocking.is_empty() {
            bail!(
                "another connection changed the source mirror of {} on the remote machine; it is mirrored again on the next build",
                blocking.join(", ")
            );
        }
        return Ok(());
    }
    for f in failed {
        let f = Path::new(f);
        // The innermost root: a root nested in a tree its enclosing root
        // excludes owns the file, and an entry in the enclosing root's
        // manifest would make that root's next diff delete it.
        let owner = st
            .roots
            .iter_mut()
            .filter(|(root, _)| f.starts_with(root))
            .max_by_key(|(root, _)| root.components().count());
        if let Some((root, r)) = owner
            && let Ok(rel) = f.strip_prefix(root)
        {
            // Present: rewrite it. Absent (a failed delete): the stale
            // entry makes the next diff delete it again.
            r.pending_repair = true;
            r.manifest.insert(rel.to_path_buf(), UNKNOWN);
        }
    }
    if ack.disk_full {
        bail!(
            "hosted build machine disk full: could not apply {} file{} (No space left on device): {}",
            failed.len(),
            if failed.len() == 1 { "" } else { "s" },
            failed
                .iter()
                .take(3)
                .cloned()
                .collect::<Vec<_>>()
                .join(", "),
        );
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
    let mut other: Vec<(String, f64)> = Vec::new();
    let single_root = roots.len() == 1;
    for root in roots {
        let Some(r) = st.roots.get(root) else {
            continue;
        };
        for rel in &r.oversized {
            let p = root.join(rel);
            let mb = std::fs::metadata(&p).map_or(0.0, |m| m.len() as f64 / 1048576.0);
            if remote_sync::is_build_source(rel) {
                src.push(format!("{} ({mb:.1} MB)", p.display()));
            } else {
                let shown = if single_root {
                    rel.display().to_string()
                } else {
                    p.display().to_string()
                };
                other.push((format!("{shown} ({mb:.1} MB)"), mb));
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
    } else if other.len() <= 2 {
        let items: Vec<&str> = other.iter().map(|(s, _)| s.as_str()).collect();
        format!(
            "not mirrored (over {} MB): {}",
            MAX_FILE >> 20,
            items.join(", ")
        )
    } else {
        let total_mb: f64 = other.iter().map(|(_, mb)| *mb).sum();
        format!(
            "not mirrored (over {} MB): {} files ({total_mb:.1} MB total; {}, {}, +{} more)",
            MAX_FILE >> 20,
            other.len(),
            other[0].0,
            other[1].0,
            other.len() - 2,
        )
    })
}

/// Bind a receipt to the same source manifests used for this submission.
/// Reattachment does not rescan, remirror, or create a new invocation.
fn source_receipt(st: &mut Sync, roots: &[PathBuf]) -> Result<String> {
    let roots = roots.iter().collect::<std::collections::BTreeSet<_>>();
    // Path equality normalizes some spellings, but the receipt hashes raw bytes.
    if let Some((key, value)) = &st.receipt
        && key
            .iter()
            .map(|p| p.as_os_str())
            .eq(roots.iter().map(|p| p.as_os_str()))
    {
        return Ok(value.clone());
    }
    let mut hash = crate::depcache::H128::new();
    hash.field(b"hosted-session-manifest-v1");
    for &root in &roots {
        hash.field(root.as_os_str().as_encoded_bytes());
        let source = st
            .roots
            .get(root)
            .context("source root has no acknowledged mirror")?;
        for (path, stamp) in &source.manifest {
            hash.field(path.as_os_str().as_encoded_bytes());
            hash.field(&stamp.mtime_ns.to_le_bytes());
            hash.field(&stamp.size.to_le_bytes());
            hash.field(&stamp.mode.to_le_bytes());
        }
        for path in &source.oversized {
            hash.field(path.as_os_str().as_encoded_bytes());
        }
    }
    let value = hash.hex();
    if roots.len() <= 32 && roots.iter().all(|p| p.as_os_str().len() <= 4096) {
        st.receipt = Some((
            roots
                .into_iter()
                .map(|p| p.as_os_str().to_owned())
                .collect(),
            value.clone(),
        ));
    }
    Ok(value)
}

#[allow(clippy::too_many_arguments)]
fn run(
    d: &Daemon,
    r: &mut BufReader<UnixStream>,
    w: &mut BufWriter<UnixStream>,
    cwd: String,
    args: Vec<String>,
    mut env: Vec<(String, String)>,
    roots: Vec<String>,
    start: bool,
    run_id: String,
    pipeline: bool,
) -> Result<()> {
    let t = Instant::now();
    let roots: Vec<PathBuf> = roots.into_iter().map(PathBuf::from).collect();
    let id = d.id();
    let (tx, rx) = channel();
    let (link, files, note, timing) = {
        let setup_phase = |w: &mut BufWriter<UnixStream>, phase: &str| -> Result<()> {
            if pipeline {
                write_frame(
                    w,
                    &Msg::RunState {
                        id,
                        may_have_started: false,
                        phase: Some(phase.into()),
                    },
                    &[],
                )?;
            }
            Ok(())
        };
        setup_phase(w, "waiting for sync lock")?;
        let st = d.sync.foreground_lock().unwrap();
        let locked = t.elapsed();
        if d.draining.load(Ordering::SeqCst) {
            bail!("sync daemon transport policy changed; reconnect before starting a build");
        }
        // The connect lets the lock go while the network is involved.
        let told = setup_phase(w, "connecting");
        let (mut st, link) = ensure_link(d, st, start, Relock::Foreground);
        let res = (|| -> Result<_> {
            told?;
            let link = link?;
            let linked = t.elapsed();
            // Only the run that waited for a connect reports its parts.
            let link_parts = st
                .link_parts
                .take()
                .filter(|_| linked - locked >= Duration::from_millis(1));
            st.recent_roots.retain(|(root, _)| !roots.contains(root));
            let now = Instant::now();
            st.recent_roots
                .extend(roots.iter().map(|root| (root.clone(), now)));
            setup_phase(w, "syncing")?;
            let mut files = repair_sources(d, &mut st, &link)?;
            for root in &roots {
                let pre = d.pre_synced.lock().unwrap().remove(root);
                let result = if st.needs_mirror(root) {
                    mirror_root(d, &mut st, &link, root)
                } else if let Some(s) = pre
                    && s.link_ptr == Arc::as_ptr(&link) as usize
                    && s.at.elapsed() < Duration::from_secs(5)
                    && root_quiet_since(&d.watcher, root, s.event_gen).is_some()
                {
                    Ok(s.files)
                } else {
                    push_root(d, &mut st, root)
                };
                match result {
                    Ok(n) => files += n,
                    Err(e) => {
                        st.source_sync_failed(root);
                        return Err(e);
                    }
                }
            }
            let pushed = t.elapsed();
            let note = oversized_note(&st, &roots)?;
            setup_phase(w, "applying sources")?;
            if !pipeline {
                barrier(d, &mut st, &link, Some(&roots))?;
            }
            if st.backend == Backend::Hosted && crate::remote_session::enabled() {
                // A correlation fingerprint of this acknowledged mirror, not
                // an authenticated content hash or a new snapshot guarantee.
                env.retain(|(key, _)| key != crate::remote_session::SOURCE_ID);
                env.push((
                    crate::remote_session::SOURCE_ID.into(),
                    source_receipt(&mut st, &roots)?,
                ));
            }
            let (barrier_tx, barrier_rx) = channel();
            {
                let mut rt = link.routes.lock().unwrap();
                if rt.dead {
                    bail!("connection to the remote machine was lost");
                }
                rt.runs.insert(id, tx);
                if pipeline {
                    rt.barriers.insert(id, barrier_tx);
                }
            }
            if pipeline {
                // Once sent upstream, a lost acknowledgement is ambiguous:
                // the client must not silently run a second local build.
                write_frame(
                    w,
                    &Msg::RunState {
                        id,
                        may_have_started: true,
                        phase: None,
                    },
                    &[],
                )?;
            }
            let msg = if pipeline {
                Msg::RunSync {
                    id,
                    cwd: cwd.clone(),
                    args,
                    env,
                    // The roots this build reads: the agent rejects it for
                    // a displaced mirror only under one of them.
                    roots: roots.iter().map(|r| r.display().to_string()).collect(),
                    start: false,
                    run_id,
                }
            } else {
                Msg::Run {
                    id,
                    cwd: cwd.clone(),
                    args,
                    env,
                    roots: Vec::new(),
                    start: false,
                    run_id,
                }
            };
            link.send(&msg, &[])?;
            if pipeline {
                let ack = match barrier_rx.recv_timeout(BARRIER_TIMEOUT) {
                    Ok(ack) => ack,
                    Err(_) => {
                        let _ = link.send(&Msg::Cancel { id }, &[]);
                        let mut rt = link.routes.lock().unwrap();
                        rt.barriers.remove(&id);
                        rt.runs.remove(&id);
                        bail!(
                            "remote machine did not acknowledge the sync; build may have started"
                        );
                    }
                };
                if let Err(e) = check_sync_failures(&mut st, &ack, Some(&roots)) {
                    // The agent confirmed it never launched this request.
                    // A local fallback is safe only after this rejection.
                    write_frame(
                        w,
                        &Msg::RunState {
                            id,
                            may_have_started: false,
                            phase: None,
                        },
                        &[],
                    )?;
                    link.routes.lock().unwrap().runs.remove(&id);
                    return Err(e);
                }
            }
            let part_ms = |i: usize| link_parts.map_or(0.0, |p| p[i].as_secs_f64() * 1000.0);
            let timing = crate::e2e::SyncTiming {
                lock_ms: locked.as_secs_f64() * 1000.0,
                link_ms: (linked - locked).as_secs_f64() * 1000.0,
                push_ms: (pushed - linked).as_secs_f64() * 1000.0,
                barrier_ms: (t.elapsed() - pushed).as_secs_f64() * 1000.0,
                link_ready_ms: part_ms(0),
                link_install_ms: part_ms(1),
                link_session_ms: part_ms(2),
            };
            if t.elapsed() > Duration::from_millis(1000) {
                log(&format!(
                    "slow sync {:.2}s: lock {:.2}s, link {:.2}s, push {:.2}s, barrier {:.2}s",
                    t.elapsed().as_secs_f64(),
                    locked.as_secs_f64(),
                    (linked - locked).as_secs_f64(),
                    (pushed - linked).as_secs_f64(),
                    (t.elapsed() - pushed).as_secs_f64()
                ));
            }
            Ok((link, files, note, timing))
        })();
        match res {
            Ok(v) => v,
            Err(e) => {
                st.receipt = None;
                // A broken pipe or lost session: start over next time.
                if let Some(link) = &st.link {
                    let _ = link.send(&Msg::Cancel { id }, &[]);
                    let mut routes = link.routes.lock().unwrap();
                    routes.runs.remove(&id);
                    routes.barriers.remove(&id);
                }
                if st.link.as_ref().is_some_and(|l| !l.alive()) {
                    drop_link(d, &mut st);
                }
                return Err(e);
            }
        }
    };
    let machine = d.info.lock().map(|i| i.machine.clone()).unwrap_or_default();
    if let Err(e) = write_frame(
        w,
        &Msg::Flushed {
            id,
            files,
            ms: t.elapsed().as_secs_f64() * 1000.0,
            machine,
            prefix: link.mirror.prefix.clone(),
            note,
            timing: Some(timing),
        },
        &[],
    ) {
        let _ = link.send(&Msg::Cancel { id }, &[]);
        link.routes.lock().unwrap().runs.remove(&id);
        return Err(e);
    }
    // A client that sends Cancel or goes away (Ctrl+C, killed agent) cancels
    // the remote run, unless it already finished.
    let finished = Arc::new(AtomicBool::new(false));
    let cr = match r.get_ref().try_clone() {
        Ok(cr) => cr,
        Err(e) => {
            let _ = link.send(&Msg::Cancel { id }, &[]);
            link.routes.lock().unwrap().runs.remove(&id);
            return Err(e.into());
        }
    };
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
    let outage_before = link.outage.restored_ms();
    while let Ok((msg, payload)) = rx.recv() {
        let done = matches!(msg, Msg::Exit { .. } | Msg::Error { .. });
        if done {
            finished.store(true, Ordering::SeqCst);
            let ms = link.outage.settled_ms() - outage_before;
            if ms > 0.0 && write_frame(w, &Msg::LinkOutage { id, ms }, &[]).is_err() {
                break;
            }
        }
        if write_frame(w, &msg, &payload).is_err() || done {
            break;
        }
    }
    Ok(())
}

// ------------------------------------------------------------- peer cache

/// Journal offsets already exchanged with the machine, so a restarted
/// daemon does not offer everything again.
#[derive(serde::Serialize, serde::Deserialize, Default, Clone, Copy, PartialEq)]
struct PeerOffsets {
    /// How far this machine's store journal was offered to the peer.
    pushed: u64,
    /// How far the peer's journal was pulled from.
    pulled: u64,
}

fn peer_offsets_file() -> Option<PathBuf> {
    crate::remote::dir()
        .ok()
        .map(|d| d.join("peer-offsets.json"))
}

/// Every few seconds while connected: offer the machine the dependency
/// entries this machine compiled since last time, and take the ones it
/// compiled, so whichever side builds next finds them. Runs only when no
/// build is relayed (pushing never competes with a remote run's output),
/// and moves the most valuable entries first. On a discharging system battery,
/// leave this optional warmup until AC returns; demand-driven fetches remain
/// available and journal offsets must not advance while warmup is deferred.
fn peer_sync_loop(d: &Daemon) {
    let file = peer_offsets_file();
    let mut off: PeerOffsets = file
        .as_ref()
        .and_then(|f| std::fs::read(f).ok())
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let mut battery_deferred = false;
    loop {
        std::thread::sleep(Duration::from_secs(5));
        if d.draining.load(Ordering::SeqCst) || d.info.lock().map_or(1, |i| i.active) > 0 {
            continue;
        }
        let on_battery = crate::sys::battery_discharging();
        if on_battery != battery_deferred {
            log(if on_battery {
                "peer cache: deferring eager transfers while system battery is discharging"
            } else {
                "peer cache: resuming eager transfers after battery discharge ended"
            });
            battery_deferred = on_battery;
        }
        if on_battery {
            continue;
        }
        let Some(relay) = d.peer.lock().ok().and_then(|c| c.clone()) else {
            continue;
        };
        if !relay.capable {
            continue;
        }
        let before = off;
        // A build may arrive after the idle check above. Recheck between
        // entries instead of letting an optional multi-GB batch monopolize
        // the connection used for source sync and barrier acknowledgements.
        let idle = || {
            !d.draining.load(Ordering::SeqCst)
                && d.info.lock().is_ok_and(|i| i.active == 0)
                && !crate::sys::battery_discharging()
        };
        let (sent, sbytes, next) = relay.push_new_while(off.pushed, idle);
        off.pushed = next;
        let (got, gbytes, next) = relay.pull_new_while(off.pulled, idle);
        off.pulled = next;
        if sent + got > 0 {
            log(&format!(
                "peer cache: pushed {sent} entries ({:.1} MB), pulled {got} ({:.1} MB)",
                sbytes as f64 / 1e6,
                gbytes as f64 / 1e6
            ));
        }
        if off != before
            && let Some(f) = &file
        {
            let _ = std::fs::write(f, serde_json::to_vec(&off).unwrap_or_default());
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    #[test]
    fn executable_install_disk_pressure_bounds_retries_and_preserves_stderr() {
        // Exercise the actual kernel ENOSPC without consuming any real disk.
        let full = || {
            remote_sync::executable_step(
                Command::new("sh")
                    .args(["-c", "LC_ALL=C; export LC_ALL; printf x | cat > /dev/full"]),
                "rsync executable upload",
            )
            .context("copying justrust to the remote machine failed")
        };
        let clock = Instant::now();
        let mut state = InstallRetry::default();
        let mut attempts = 0;
        for second in 0..38 {
            let err = state
                .run(
                    "a",
                    || clock + Duration::from_secs(second),
                    || {
                        attempts += 1;
                        full()
                    },
                )
                .unwrap_err();
            let diagnostic = format!("{err:#}");
            assert!(
                diagnostic.contains("No space left on device"),
                "{diagnostic}"
            );
            assert!(diagnostic.contains("rsync executable upload"));
            assert!(diagnostic.contains("copying justrust"));
        }
        assert_eq!(attempts, 1);
        // Expiration, a new machine, and non-capacity errors must not pin failures.
        state
            .run("a", || clock + Duration::from_secs(60), || Ok(()))
            .unwrap();
        state.run("a", || clock, full).unwrap_err();
        state
            .run(
                "b",
                || clock,
                || -> Result<()> { anyhow::bail!("broken pipe") },
            )
            .unwrap_err();
        state.run("b", || clock, || Ok(())).unwrap();
    }

    #[test]
    fn mirror_observation_requires_both_preparation_and_scan() {
        let temp = std::env::temp_dir().join(format!("jr-prepare-observe-{}", std::process::id()));
        let mirror = Mirror {
            target: None,
            prefix: String::new(),
        };
        let root = temp.join("mirror");
        let (value, _, _) = prepare_observing(&mirror, &root, || Ok(42)).unwrap();
        assert!(root.is_dir());
        assert_eq!(value, 42);
        let err = prepare_observing(&mirror, &root, || -> Result<()> {
            anyhow::bail!("scan failed")
        })
        .unwrap_err();
        assert_eq!(err.to_string(), "scan failed");
        let file = temp.join("not-a-directory");
        std::fs::write(&file, []).unwrap();
        assert!(prepare_observing(&mirror, &file, || Ok(())).is_err());
        std::fs::remove_dir_all(temp).unwrap();
    }

    #[test]
    #[ignore = "compares local source scan with live hosted receiver preparation"]
    fn hosted_mirror_setup_benchmark() {
        let root = std::env::current_dir().unwrap();
        let mirror = Mirror {
            target: Some(
                crate::remote_backend::Backend::Hosted
                    .ensure_ready()
                    .unwrap(),
            ),
            prefix: String::new(),
        };
        let mut times = [Vec::new(), Vec::new()];
        for i in 0..12 {
            let parallel = [false, true, true, false][i % 4];
            let start = Instant::now();
            let scan = if parallel {
                prepare_observing(&mirror, &root, || remote_sync::scan(&root))
                    .unwrap()
                    .0
            } else {
                mirror.prepare(&root).unwrap();
                remote_sync::scan(&root).unwrap()
            };
            assert!(!scan.files.is_empty());
            times[usize::from(parallel)].push(start.elapsed().as_millis());
        }
        eprintln!(
            "mirror setup ms serial={:?} overlapped={:?}",
            times[0], times[1]
        );
    }
    #[test]
    fn binary_identity_tracks_bytes_not_copy_metadata() {
        let base = std::env::temp_dir().join(format!("jr-binary-id-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let first = base.join("first");
        let copy = base.join("copy");
        std::fs::write(&first, vec![1u8; 150_000]).unwrap();
        std::fs::copy(&first, &copy).unwrap();
        let old = std::time::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let new = old + Duration::from_secs(60);
        let set_time = |path: &Path, time| {
            std::fs::OpenOptions::new()
                .write(true)
                .open(path)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(time))
                .unwrap();
        };
        let identity =
            |path: &Path| binary_identity(std::fs::File::open(path).and_then(binary_digest));
        set_time(&first, old);
        set_time(&copy, new);
        assert_ne!(
            std::fs::metadata(&first).unwrap().modified().unwrap(),
            std::fs::metadata(&copy).unwrap().modified().unwrap()
        );
        assert_eq!(identity(&first), identity(&copy));
        std::fs::write(&copy, vec![2u8; 150_000]).unwrap();
        set_time(&copy, old);
        assert_eq!(
            std::fs::metadata(&first).unwrap().len(),
            std::fs::metadata(&copy).unwrap().len()
        );
        assert_eq!(
            std::fs::metadata(&first).unwrap().modified().unwrap(),
            std::fs::metadata(&copy).unwrap().modified().unwrap()
        );
        assert_ne!(identity(&first), identity(&copy));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn stored_digest_is_used_only_for_the_same_file_state() {
        let base = std::env::temp_dir().join(format!("jr-digest-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let cache = base.join("build-id");
        let exe = base.join("exe");
        let digest =
            |path: &Path| file_digest(Some(&cache), std::fs::File::open(path).unwrap()).unwrap();
        let plain = |path: &Path| binary_digest(std::fs::File::open(path).unwrap()).unwrap();
        let entries = || -> Vec<PathBuf> {
            std::fs::read_dir(&cache)
                .map(|rd| rd.flatten().map(|e| e.path()).collect())
                .unwrap_or_default()
        };

        std::fs::write(&exe, vec![1u8; 150_000]).unwrap();
        let first = digest(&exe);
        assert_eq!(first, plain(&exe));
        let stored = entries();
        assert_eq!(stored.len(), 1, "one entry and no temporary file");

        // Proof the entry is what answers: a planted digest under the same
        // key is returned without reading the file.
        let key = digest_key(&std::fs::File::open(&exe).unwrap()).unwrap();
        let planted = format!("sha256-{}", "ab".repeat(32));
        std::fs::write(&stored[0], format!("{key} {planted}\n")).unwrap();
        assert_eq!(digest(&exe), planted);

        // A damaged entry is ignored and replaced.
        std::fs::write(&stored[0], format!("{key} sha256-short\n")).unwrap();
        assert_eq!(digest(&exe), first);
        std::fs::write(&stored[0], format!("{key} {first}")).unwrap();
        assert_eq!(digest(&exe), first);
        assert_eq!(stored_digest(&stored[0], &key), Some(first.clone()));

        // Same inode, size and modification time, other bytes: the planted
        // digest of the old state must not answer.
        let mtime = std::fs::metadata(&exe).unwrap().modified().unwrap();
        std::fs::write(&stored[0], format!("{key} {planted}\n")).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(&exe, vec![2u8; 150_000]).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&exe)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(mtime))
            .unwrap();
        assert_eq!(std::fs::metadata(&exe).unwrap().modified().unwrap(), mtime);
        let second = digest(&exe);
        assert_eq!(second, plain(&exe));
        assert_ne!(second, first);

        // An installer's replacement is another inode with its own entry.
        let next = base.join("next");
        std::fs::write(&next, vec![3u8; 150_000]).unwrap();
        std::fs::rename(&next, &exe).unwrap();
        assert_eq!(digest(&exe), plain(&exe));

        // Old entries go once there are many, the one just written stays.
        let kept = entries().len();
        assert!(kept >= 2);
        let mine = entries().into_iter().next().unwrap();
        prune_digests(&cache, &mine, kept, Duration::ZERO);
        assert_eq!(entries().len(), kept, "nothing goes below the limit");
        prune_digests(&cache, &mine, 1, Duration::ZERO);
        assert_eq!(entries(), vec![mine]);

        // No usable cache directory: hash, and say nothing.
        let blocked = base.join("file");
        std::fs::write(&blocked, b"not a directory").unwrap();
        assert_eq!(
            file_digest(Some(&blocked), std::fs::File::open(&exe).unwrap()).unwrap(),
            plain(&exe)
        );
        assert_eq!(
            file_digest(None, std::fs::File::open(&exe).unwrap()).unwrap(),
            plain(&exe)
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn an_agent_that_never_answers_is_stopped_and_one_that_answers_is_not() {
        let silent = || {
            Command::new("sleep")
                .arg("600")
                .stdout(Stdio::piped())
                .spawn()
                .unwrap()
        };
        // Never answers: the blocked read ends once the bound passes.
        let mut child = silent();
        let mut out = child.stdout.take().unwrap();
        let greeted = Arc::new(AtomicBool::new(false));
        let watch = {
            let (greeted, pid) = (greeted.clone(), child.id() as i32);
            std::thread::spawn(move || {
                stop_unless_greeted(pid, &greeted, Duration::from_millis(300))
            })
        };
        let start = Instant::now();
        let mut buf = [0u8; 1];
        assert_eq!(out.read(&mut buf).unwrap(), 0);
        assert!(start.elapsed() < Duration::from_secs(20));
        assert!(watch.join().unwrap());
        assert!(greeted.load(Ordering::SeqCst));
        assert!(!child.wait().unwrap().success());
        // Answered in time: left alone.
        let mut child = silent();
        let greeted = AtomicBool::new(true);
        assert!(!stop_unless_greeted(
            child.id() as i32,
            &greeted,
            Duration::from_millis(300)
        ));
        assert!(child.try_wait().unwrap().is_none());
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn binary_identity_read_failure_never_uses_partial_or_shared_digest() {
        struct Broken;
        impl Read for Broken {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::other("unreadable binary"))
            }
        }
        let first = binary_identity(binary_digest(Broken));
        assert!(first.starts_with("unreadable-"));
        assert_ne!(first, binary_identity(binary_digest(Broken)));
        assert_eq!(
            binary_digest(&b"abc"[..]).unwrap(),
            "sha256-ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(build_id(), build_id());
    }

    #[test]
    #[ignore = "explicit hashing cost measurement"]
    fn binary_identity_hashing_cost() {
        let executable = std::env::current_exe().unwrap();
        let bytes = std::fs::metadata(&executable).unwrap().len();
        let start = Instant::now();
        let expected = binary_digest(std::fs::File::open(&executable).unwrap()).unwrap();
        let elapsed = start.elapsed();
        let cached = build_id();
        assert_eq!(expected, cached);
        let start = Instant::now();
        for _ in 0..10_000 {
            std::hint::black_box(build_id());
        }
        eprintln!(
            "binary identity: {bytes} bytes, streaming hash {elapsed:?}, 10000 cached calls {:?}, buffer 65536 bytes",
            start.elapsed()
        );
    }
    #[cfg(unix)]
    #[test]
    fn daemon_startup_waits_for_shutdown_lock_after_socket_closes() {
        let base = std::env::temp_dir().join(format!("jr-shutdown-handoff-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("daemon.lock");
        let old = std::fs::File::create(&path).unwrap();
        old.try_lock().unwrap();
        let replacement = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let (client, server) = UnixStream::pair().unwrap();
        drop(server);
        let mut byte = [0];
        assert_eq!((&client).read(&mut byte).unwrap(), 0);

        // No sleeps or scheduler assumptions: retirement releases its lock
        // only once startup has observed contention after socket EOF.
        let mut waits = 0;
        let ready = wait_for_daemon_lock(
            &replacement,
            Duration::from_secs(30),
            || None,
            || {
                waits += 1;
                old.unlock().unwrap();
            },
        )
        .unwrap();
        assert!(ready.is_none());
        assert_eq!(waits, 1);
        assert!(matches!(
            old.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));
        replacement.unlock().unwrap();
        drop((old, replacement));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn daemon_startup_lock_wait_is_bounded_and_reuses_concurrent_replacement() {
        let base = std::env::temp_dir().join(format!("jr-startup-bounded-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("daemon.lock");
        let owner = std::fs::File::create(&path).unwrap();
        owner.try_lock().unwrap();
        let contender = std::fs::OpenOptions::new().write(true).open(&path).unwrap();
        let error = wait_for_daemon_lock(
            &contender,
            Duration::ZERO,
            || None,
            || panic!("must not wait beyond deadline"),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("shutdown still holds daemon.lock")
        );
        assert!(matches!(
            contender.try_lock(),
            Err(std::fs::TryLockError::WouldBlock)
        ));

        let (client, _server) = UnixStream::pair().unwrap();
        let mut client = Some(client);
        assert!(
            wait_for_daemon_lock(
                &contender,
                Duration::ZERO,
                || client.take(),
                || panic!("healthy replacement must not wait"),
            )
            .unwrap()
            .is_some()
        );
        owner.unlock().unwrap();
        drop((owner, contender));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn daemon_transport_policy_identity_invalidates_mode_changes_not_aliases() {
        let identity = |mode: Option<&str>, only: Option<&str>| {
            policy_identity("binary", |key| match key {
                "JUSTRUST_HOSTED_NATIVE" => mode.map(str::to_owned),
                "JUSTRUST_HOSTED_NATIVE_ONLY" => only.map(str::to_owned),
                _ => None,
            })
        };
        let off = identity(Some("off"), None);
        let required = identity(Some("require"), None);
        let auto = identity(None, None);
        assert_ne!(off, required);
        assert_ne!(off, auto);
        assert_ne!(required, auto);
        for alias in ["0", "off", "false"] {
            assert_eq!(off, identity(Some(alias), Some("require")));
        }
        for alias in ["1", "on", "true", "require"] {
            assert_eq!(required, identity(Some(alias), None));
            assert_eq!(identity(None, Some("require")), identity(None, Some(alias)));
        }
        assert_ne!(required, identity(Some("require"), Some("true")));
        assert_ne!(auto, identity(None, Some("require")));
        // A pre-policy daemon's binary-only pong must also be retired.
        assert_ne!(off, "binary");
        assert_ne!(required, "binary");
        assert_ne!(
            policy_identity("", |_| None),
            policy_identity("", |_| Some("off".into()))
        );
    }

    #[test]
    fn daemon_mirror_isolation_policy_requires_explicit_opt_in_and_distinct_identity() {
        let identity = |value: Option<&str>| {
            policy_identity("binary", |key| {
                if key == "JUSTRUST_HOSTED_MIRROR_ISOLATION" {
                    value.map(str::to_owned)
                } else {
                    None
                }
            })
        };
        let shared = identity(None);
        let isolated = identity(Some("1"));
        assert_ne!(shared, isolated);
        for value in [None, Some("0"), Some("off"), Some("false"), Some("unknown")] {
            assert!(!hosted_mirror_isolation(value));
            assert_eq!(shared, identity(value));
        }
        for value in ["1", "on", "true", "require"] {
            assert!(hosted_mirror_isolation(Some(value)));
            assert_eq!(isolated, identity(Some(value)));
        }
    }

    #[cfg(unix)]
    #[test]
    fn remote_disconnect_retains_bounded_cause_for_waiting_run() {
        for (script, expected) in [
            ("exit 0", "connection_closed"),
            ("printf '\\377\\377\\377\\377'", "protocol"),
        ] {
            let mut child = Command::new("sh")
                .args(["-c", script])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let stdin = Arc::new(Mutex::new(BufWriter::new(child.stdin.take().unwrap())));
            let stdout = BufReader::new(child.stdout.take().unwrap());
            let link = Link {
                outage: Arc::default(),
                child: Mutex::new(child),
                stdin,
                routes: Mutex::default(),
                peer: peercache::Relay::new(true, |_, _| true),
                checkpoint: false,
                mirror: Mirror {
                    target: None,
                    prefix: String::new(),
                },
            };
            let (tx, rx) = channel();
            link.routes.lock().unwrap().runs.insert(77, tx);
            read_agent(&link, stdout, None);
            let (message, payload) = rx.recv().unwrap();
            match message {
                Msg::Error { id, msg } => {
                    assert_eq!(id, 77);
                    assert_eq!(
                        msg,
                        format!("connection to the remote machine was lost: {expected}")
                    );
                    assert_eq!(crate::remote_build::transport_failure_kind(&msg), expected);
                }
                other => panic!("unexpected disconnect message: {other:?}"),
            }
            assert!(payload.is_empty());
            assert!(link.routes.lock().unwrap().dead);
            link.child.lock().unwrap().wait().unwrap();
        }
    }

    #[cfg(unix)]
    #[test]
    fn daemon_transport_policy_drain_preserves_active_route_without_resubmission() {
        let base = std::env::temp_dir().join(format!("jr-policy-drain-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let sock = base.join("daemon.sock");
        let listener = UnixListener::bind(&sock).unwrap();
        let md = std::fs::metadata(&sock).unwrap();
        let lock_path = base.join("daemon.lock");
        let lock = std::fs::File::create(&lock_path).unwrap();
        lock.try_lock().unwrap();
        // Model the descriptor retained across a concurrent fork. This keeps
        // close-only retirement locked deterministically on Unix, rather than
        // relying on another test's short fork-to-exec scheduling window.
        let inherited_lock = lock.try_clone().unwrap();
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = Arc::new(Mutex::new(BufWriter::new(child.stdin.take().unwrap())));
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let link = Arc::new(Link {
            outage: Arc::default(),
            child: Mutex::new(child),
            stdin,
            routes: Mutex::default(),
            peer: peercache::Relay::new(true, |_, _| true),
            checkpoint: false,
            mirror: Mirror {
                target: None,
                prefix: String::new(),
            },
        });
        let reader_link = link.clone();
        let reader = std::thread::spawn(move || read_agent(&reader_link, stdout, None));
        let (tx, rx) = channel();
        link.routes.lock().unwrap().runs.insert(77, tx);
        let d = Daemon {
            status_hint: None,
            sync: SyncQueue::new(Sync {
                install_retry: Default::default(),
                link_parts: None,
                recent_roots: Vec::new(),
                receipt: None,
                backend: Backend::Hosted,
                link: Some(link.clone()),
                roots: BTreeMap::new(),
            }),
            info: Mutex::new(Info {
                active: 1,
                ..Default::default()
            }),
            next_id: AtomicU64::new(100),
            watcher: Watcher::new(channel().0).unwrap(),
            sock: (sock.clone(), crate::sys::file_id(&md)),
            lock: Mutex::new(Some(lock)),
            draining: AtomicBool::new(false),
            peer: Arc::default(),
            peer_sock: Mutex::default(),
            hosted_agent: hosted_admit_agent(),
            pre_synced: Mutex::default(),
            live_link_ptr: AtomicUsize::new(0),
            warm_admit: Mutex::default(),
            leases: Arc::default(),
            relink: None,
        };
        // Simulate a sync already in progress. Retirement must not wait on it
        // or replace its link, kill its process, cancel, or resubmit its run.
        let held = d.sync.lock().unwrap();
        assert!(d.begin_draining(1));
        assert!(!d.begin_draining(1));
        assert!(!sock.exists());
        assert!(
            std::fs::OpenOptions::new()
                .write(true)
                .open(&lock_path)
                .unwrap()
                .try_lock()
                .is_ok()
        );
        drop(inherited_lock);
        assert!(Arc::ptr_eq(held.link.as_ref().unwrap(), &link));
        drop(held);
        let (client, server) = UnixStream::pair().unwrap();
        let mut r = BufReader::new(server.try_clone().unwrap());
        let mut w = BufWriter::new(server);
        let error = run(
            &d,
            &mut r,
            &mut w,
            "/".into(),
            vec![],
            vec![],
            vec![],
            false,
            "new-run".into(),
            true,
        )
        .unwrap_err();
        assert!(error.to_string().contains("transport policy changed"));
        assert_eq!(link.routes.lock().unwrap().runs.len(), 1);
        assert!(link.alive());
        assert!(link.child.lock().unwrap().try_wait().unwrap().is_none());
        link.send(
            &Msg::Exit {
                id: 77,
                code: 0,
                timing: None,
            },
            &[],
        )
        .unwrap();
        assert!(matches!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap().0,
            Msg::Exit {
                id: 77,
                code: 0,
                ..
            }
        ));
        assert!(rx.try_recv().is_err());
        link.kill();
        reader.join().unwrap();
        drop(client);
        drop(listener);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn status_hint_tracks_only_displayed_changes() {
        let mut info = Info::default();
        assert!(!info.update(|_| {}));
        assert!(!info.update(|i| {
            i.last_client = Some(Instant::now());
            i.active += 1;
            i.skipped += 1;
        }));
        assert!(info.update(|i| i.roots.push("/workspace".into())));
        assert!(!info.update(|i| i.roots = vec!["/workspace".into()]));
        assert!(info.update(|i| i.machine = "hosted".into()));
        assert!(!info.update(|i| i.machine = "hosted".into()));
        assert!(info.update(|i| i.pushed += 1));
        assert!(info.update(|i| i.connected = true));
        assert!(!info.update(|i| i.connected = true));
        assert!(info.update(|i| i.connected = false));
    }

    #[cfg(unix)]
    #[test]
    fn status_hint_is_private_and_rejects_links() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let base = std::env::temp_dir().join(format!("jr-status-hint-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let path = base.join("daemon-status");
        write_status_hint(&path);
        assert_eq!(std::fs::read(&path).unwrap(), b"1\n");
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        write_status_hint(&path);
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        let target = base.join("target");
        std::fs::write(&target, b"untouched").unwrap();
        std::fs::remove_file(&path).unwrap();
        symlink(&target, &path).unwrap();
        write_status_hint(&path);
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
        std::fs::remove_file(&path).unwrap();
        std::fs::hard_link(&target, &path).unwrap();
        write_status_hint(&path);
        assert_eq!(std::fs::read(&target).unwrap(), b"untouched");
        std::fs::remove_dir_all(base).unwrap();
    }

    use super::*;

    /// A file rewritten after the scan, even to the same size and with its
    /// old mtime restored (`shutil.copy2`), is not sent under the scanned
    /// stamp: that stamp would match again later and the bytes on the
    /// machine would never be corrected.
    #[test]
    fn push_reads_only_the_file_the_scan_stamped() {
        let d = std::env::temp_dir().join(format!("jr-settled-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("api_keys.rs");
        std::fs::write(&p, "pub fn f() {}\n").unwrap();
        let stamp = |p: &Path| {
            let m = std::fs::symlink_metadata(p).unwrap();
            let (s, ns) = crate::sys::mtime(&m);
            remote_sync::Stamp {
                mtime_ns: s as i128 * 1_000_000_000 + ns as i128,
                size: m.len(),
                mode: 0o644,
            }
        };
        let scanned = stamp(&p);
        assert_eq!(
            read_settled(&p, Some(scanned)).as_deref(),
            Some(&b"pub fn f() {}\n"[..])
        );
        // Truncated mid-copy: size differs from the stamp.
        std::fs::write(&p, "").unwrap();
        assert!(read_settled(&p, Some(scanned)).is_none());
        // Rewritten with different content: mtime differs.
        std::thread::sleep(std::time::Duration::from_millis(5));
        std::fs::write(&p, "pub fn g() {}\n").unwrap();
        assert!(read_settled(&p, Some(scanned)).is_none());
        assert!(read_settled(&p, None).is_none());
        assert!(read_settled(&d.join("gone.rs"), Some(scanned)).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    #[ignore = "focused real-manifest watch-directory reconstruction diagnostic"]
    fn representative_watch_directory_reuse() {
        let root = std::env::var_os("JUSTRUST_SCAN_BENCH_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));
        let manifest = remote_sync::scan(&root).unwrap().files;
        let expected = remote_sync::dirs(&root, &manifest);
        let mut tracked = Root {
            pending_repair: false,
            manifest: manifest.clone(),
            watch_dirs: Some(expected.clone()),
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        let diff = remote_sync::Diff::default();
        for iteration in 0..5 {
            let start = Instant::now();
            let rebuilt = std::hint::black_box(remote_sync::dirs(&root, &manifest));
            let rebuild_ns = start.elapsed().as_nanos();
            assert_eq!(rebuilt, expected);
            let start = Instant::now();
            let cached = std::hint::black_box(tracked.watch_directories(&root, &manifest, &diff));
            let reuse_ns = start.elapsed().as_nanos();
            assert_eq!(cached, &expected);
            eprintln!(
                "watch dirs {iteration}: rebuild_ns={rebuild_ns} reuse_ns={reuse_ns} files={} dirs={}",
                manifest.len(),
                expected.len()
            );
        }
    }

    #[test]
    fn cached_watch_directories_follow_paths_not_stamps() {
        let root = Path::new("/workspace");
        let mut tracked = Root {
            pending_repair: true,
            manifest: Manifest::from([(PathBuf::from("src/lib.rs"), UNKNOWN)]),
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        for paths in [
            vec!["src/lib.rs"],
            vec!["src/lib.rs"],
            vec!["src/lib.rs", "new/deep/file.rs"],
            vec!["other/file.rs"],
            vec![],
            vec!["src/lib.rs"],
        ] {
            let next: Manifest = paths
                .into_iter()
                .map(|p| {
                    (
                        PathBuf::from(p),
                        remote_sync::Stamp {
                            mtime_ns: 123,
                            size: 7,
                            mode: 0o644,
                        },
                    )
                })
                .collect();
            let diff = remote_sync::diff(&tracked.manifest, &next);
            let expected = remote_sync::dirs(root, &next);
            assert_eq!(tracked.watch_directories(root, &next, &diff), &expected);
            tracked.manifest = next;
        }
    }

    #[test]
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn cached_watch_directories_retry_lost_registration() {
        let root = std::env::temp_dir().join(format!("jr-watch-cache-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let mut tracked = Root {
            pending_repair: true,
            manifest: Manifest::from([(PathBuf::from("src/lib.rs"), UNKNOWN)]),
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        let (tx, _rx) = channel();
        let watcher = Watcher::new(tx).unwrap();
        let next = tracked.manifest.clone();
        let diff = remote_sync::diff(&tracked.manifest, &next);
        assert_eq!(
            watcher.watch_all(tracked.watch_directories(&root, &next, &diff)),
            2
        );
        assert_eq!(
            watcher.watch_all(tracked.watch_directories(&root, &next, &diff)),
            0
        );
        // A new watcher has no registrations but can reuse exactly the same
        // cached paths. No absence of filesystem events is trusted.
        let (tx, _rx) = channel();
        let replacement = Watcher::new(tx).unwrap();
        assert_eq!(
            replacement.watch_all(tracked.watch_directories(&root, &next, &diff)),
            2
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[ignore = "repair root lookup microbenchmark"]
    fn benchmark_repair_root_lookup() {
        use std::hint::black_box;
        let manifest: Manifest = (0..8192)
            .map(|i| {
                (
                    PathBuf::from(format!("crates/c{}/src/module{i}.rs", i / 128)),
                    remote_sync::Stamp {
                        mtime_ns: i,
                        size: 8192,
                        mode: 0o644,
                    },
                )
            })
            .collect();
        for count in [1, 20] {
            let mut sync = Sync {
                install_retry: Default::default(),
                link_parts: None,
                recent_roots: Vec::new(),
                receipt: None,
                backend: Backend::Hosted,
                link: None,
                roots: (0..count)
                    .map(|i| {
                        (
                            PathBuf::from(format!("/workspace/{i}")),
                            Root {
                                pending_repair: false,
                                manifest: manifest.clone(),
                                watch_dirs: None,
                                oversized: vec![],
                                needs_full_sync: false,
                                mark: Default::default(),
                                clean_event_gen: None,
                            },
                        )
                    })
                    .collect(),
            };
            let old = |s: &Sync| {
                s.roots
                    .iter()
                    .filter(|(_, r)| r.manifest.values().any(|v| *v == UNKNOWN))
                    .map(|(p, _)| p.clone())
                    .collect::<Vec<_>>()
            };
            for rejected in [false, true] {
                if rejected {
                    let failed = [0, count - 1]
                        .map(|i| format!("/workspace/{i}/z-failed-delete"))
                        .to_vec();
                    assert!(
                        check_sync_failures(
                            &mut sync,
                            &SyncAck {
                                failed,
                                ..Default::default()
                            },
                            None,
                        )
                        .is_err()
                    );
                }
                assert_eq!(old(&sync), sync.repair_roots());
                let t = Instant::now();
                for _ in 0..64 {
                    black_box(old(black_box(&sync)));
                }
                let old_ns = t.elapsed().as_nanos() / 64;
                let t = Instant::now();
                for _ in 0..8192 {
                    black_box(black_box(&sync).repair_roots());
                }
                eprintln!(
                    "{count} roots x 8192 files, rejected={rejected}: old {old_ns} ns, indexed {} ns",
                    t.elapsed().as_nanos() / 8192
                );
            }
        }
    }

    #[test]
    fn disk_recovery_resends_other_roots_exact_failed_writes_and_deletes() {
        let base = std::env::temp_dir().join(format!("jr-disk-repair-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        std::fs::write(a.join("lib.rs"), b"current A source").unwrap();
        std::fs::write(a.join("unchanged.rs"), b"unchanged A").unwrap();
        std::fs::write(b.join("lib.rs"), b"current B source").unwrap();
        let make_root = |path: &Path| Root {
            pending_repair: false,
            manifest: remote_sync::scan(path).unwrap().files,
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut frames = BufReader::new(child.stdout.take().unwrap());
        let link = Arc::new(Link {
            outage: Arc::default(),
            stdin: Arc::new(Mutex::new(BufWriter::new(child.stdin.take().unwrap()))),
            child: Mutex::new(child),
            routes: Mutex::default(),
            peer: peercache::Relay::new(false, |_, _| true),
            checkpoint: false,
            mirror: Mirror {
                target: None,
                prefix: String::new(),
            },
        });
        let d = Daemon {
            status_hint: None,
            sync: SyncQueue::new(Sync {
                install_retry: Default::default(),
                link_parts: None,
                recent_roots: Vec::new(),
                receipt: None,
                backend: Backend::Hosted,
                link: Some(link.clone()),
                roots: BTreeMap::from([(a.clone(), make_root(&a)), (b.clone(), make_root(&b))]),
            }),
            info: Mutex::default(),
            next_id: AtomicU64::new(1),
            watcher: Watcher::new(channel().0).unwrap(),
            sock: (base.join("unused.sock"), (0, 0)),
            lock: Mutex::new(None),
            draining: AtomicBool::new(false),
            peer: Arc::default(),
            peer_sock: Mutex::default(),
            hosted_agent: hosted_admit_agent(),
            pre_synced: Mutex::default(),
            live_link_ptr: AtomicUsize::new(0),
            warm_admit: Mutex::default(),
            leases: Arc::default(),
            relink: None,
        };
        let mut st = d.sync.lock().unwrap();
        let failed_write = a.join("lib.rs").display().to_string();
        let failed_delete = a.join("gone.rs").display().to_string();
        let full = SyncAck {
            failed: vec![failed_write.clone(), failed_delete.clone()],
            disk_full: true,
            lost: Vec::new(),
        };
        assert!(check_sync_failures(&mut st, &full, None).is_err());
        assert_eq!(st.repair_roots(), vec![a.clone()]);
        // B is the next requested workspace. Repair A first, even though A
        // has no new local changes, and do not resend unrelated source.
        assert_eq!(repair_sources(&d, &mut st, &link).unwrap(), 2);
        assert_eq!(push_root(&d, &mut st, &b).unwrap(), 0);
        assert_eq!(
            read_frame(&mut frames).unwrap().unwrap(),
            (
                Msg::Write {
                    path: failed_write.clone(),
                    mode: st.roots[&a].manifest[Path::new("lib.rs")].mode
                },
                b"current A source".to_vec()
            )
        );
        assert_eq!(
            read_frame(&mut frames).unwrap().unwrap(),
            (
                Msg::Delete {
                    path: failed_delete
                },
                Vec::new()
            )
        );
        assert!(st.repair_roots().is_empty());
        // Sending retries is not proof of remote success. An outstanding
        // ENOSPC receipt still rejects the build and schedules them again.
        assert!(check_sync_failures(&mut st, &full, None).is_err());
        assert_eq!(repair_sources(&d, &mut st, &link).unwrap(), 2);
        assert!(check_sync_failures(&mut st, &SyncAck::default(), None).is_ok());
        assert!(st.repair_roots().is_empty());
        assert_eq!(repair_sources(&d, &mut st, &link).unwrap(), 0);
        // A unavailable or changed into a non-regular source must not be
        // interpreted as a successful repair, or authorize B on stale A.
        let receipt = SyncAck {
            failed: vec![failed_write],
            disk_full: false,
            lost: Vec::new(),
        };
        assert!(check_sync_failures(&mut st, &receipt, None).is_err());
        std::fs::remove_file(a.join("lib.rs")).unwrap();
        std::fs::create_dir(a.join("lib.rs")).unwrap();
        assert!(repair_sources(&d, &mut st, &link).is_err());
        assert_eq!(st.repair_roots(), vec![a.clone()]);
        std::fs::rename(&a, base.join("unavailable-a")).unwrap();
        assert!(repair_sources(&d, &mut st, &link).is_err());
        assert_eq!(st.repair_roots(), vec![a]);
        link.kill();
        drop(st);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn reconnect_keeps_a_confirmed_mirror_and_resends_only_unsettled_paths() {
        let base = std::env::temp_dir().join(format!("jr-reverify-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("w");
        std::fs::create_dir_all(&root).unwrap();
        for name in ["a.rs", "b.rs", "c.rs"] {
            std::fs::write(root.join(name), name).unwrap();
        }
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut frames = BufReader::new(child.stdout.take().unwrap());
        let link = Arc::new(Link {
            outage: Arc::default(),
            stdin: Arc::new(Mutex::new(BufWriter::new(child.stdin.take().unwrap()))),
            child: Mutex::new(child),
            routes: Mutex::default(),
            peer: peercache::Relay::new(false, |_, _| true),
            checkpoint: true,
            mirror: Mirror {
                target: None,
                prefix: String::new(),
            },
        });
        let tracked = Root {
            pending_repair: false,
            manifest: remote_sync::scan(&root).unwrap().files,
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        let d = Daemon {
            status_hint: None,
            sync: SyncQueue::new(Sync {
                install_retry: Default::default(),
                link_parts: None,
                recent_roots: Vec::new(),
                receipt: None,
                backend: Backend::Hosted,
                link: Some(link.clone()),
                roots: BTreeMap::from([(root.clone(), tracked)]),
            }),
            info: Mutex::default(),
            next_id: AtomicU64::new(1),
            watcher: Watcher::new(channel().0).unwrap(),
            sock: (base.join("unused.sock"), (0, 0)),
            lock: Mutex::new(None),
            draining: AtomicBool::new(false),
            peer: Arc::default(),
            peer_sock: Mutex::default(),
            hosted_agent: hosted_admit_agent(),
            pre_synced: Mutex::default(),
            live_link_ptr: AtomicUsize::new(0),
            warm_admit: Mutex::default(),
            leases: Arc::default(),
            relink: None,
        };
        let mut st = d.sync.lock().unwrap();
        let name = root.display().to_string();
        let path = |rel: &str| root.join(rel).display().to_string();
        st.roots.get_mut(&root).unwrap().mark.hashes =
            snapshot_hashes(&root, &st.roots[&root].manifest);
        // Model a source mutation after the copy's source snapshot but before
        // Checkpoint. Even metadata restored by cp -p must not redefine the
        // remote snapshot's expected content hash.
        let c = root.join("c.rs");
        let modified = std::fs::metadata(&c).unwrap().modified().unwrap();
        std::fs::write(&c, b"diff").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&c)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();

        // In sync and nothing to send: the agent is asked to remember that.
        assert_eq!(push_root(&d, &mut st, &root).unwrap(), 0);
        let first = match read_frame(&mut frames).unwrap().unwrap().0 {
            Msg::Checkpoint { path, token } if path == name => token,
            other => panic!("expected Checkpoint, got {other:?}"),
        };
        let hash = st
            .roots
            .get_mut(&root)
            .unwrap()
            .mark
            .hashes
            .remove(Path::new("c.rs"))
            .unwrap();
        assert!(
            reverify(&d, &mut st, &link, &root).is_none(),
            "missing source snapshot evidence must force a full mirror"
        );
        st.roots
            .get_mut(&root)
            .unwrap()
            .mark
            .hashes
            .insert(PathBuf::from("c.rs"), hash);
        assert_eq!(st.roots[&root].mark.token.as_deref(), Some(first.as_str()));

        // Later frames are recorded, not checkpointed one by one.
        std::fs::write(root.join("a.rs"), "edited").unwrap();
        std::fs::remove_file(root.join("b.rs")).unwrap();
        assert_eq!(push_root(&d, &mut st, &root).unwrap(), 2);
        assert!(matches!(
            read_frame(&mut frames).unwrap().unwrap().0,
            Msg::Write { path: p, .. } if p == path("a.rs")
        ));
        assert!(matches!(
            read_frame(&mut frames).unwrap().unwrap().0,
            Msg::Delete { path: p } if p == path("b.rs")
        ));
        assert_eq!(st.roots[&root].mark.dirty.len(), 2);
        assert_eq!(st.roots[&root].mark.token.as_deref(), Some(first.as_str()));

        // The connection is replaced. An agent that confirms the checkpoint
        // is asked only about files that were settled when it was taken.
        let answer = |ok: bool, mut frames: BufReader<std::process::ChildStdout>| {
            let link = link.clone();
            std::thread::spawn(move || {
                let (msg, payload) = read_frame(&mut frames).unwrap().unwrap();
                let Msg::Verify {
                    id, token, next, ..
                } = msg
                else {
                    panic!("expected Verify, got {msg:?}");
                };
                let mut raw = Vec::new();
                flate2::read::GzDecoder::new(&payload[..])
                    .read_to_end(&mut raw)
                    .unwrap();
                let tx = link.routes.lock().unwrap().verifies.remove(&id).unwrap();
                tx.send(if ok { Ok(()) } else { Err("size".into()) })
                    .unwrap();
                (frames, token, next, raw)
            })
        };
        st.invalidate_sources();
        assert!(st.needs_mirror(&root));
        // Mutation after checkpoint must not become the expected remote hash.
        // This also proves reconnect does not hash the current local tree.
        std::fs::write(root.join("c.rs"), b"oops").unwrap();
        let agent = answer(true, frames);
        assert_eq!(mirror_root(&d, &mut st, &link, &root).unwrap(), 3);
        let (mut frames, token, next, list) = agent.join().unwrap();
        assert_eq!(token, first);
        assert_eq!(
            list,
            format!("4\0{}\0c.rs\0", remote_checkpoint::digest(b"c.rs")).as_bytes()
        );
        assert!(!st.needs_mirror(&root));
        assert_eq!(st.roots[&root].mark.token.as_deref(), Some(next.as_str()));
        // What the old connection may not have applied goes again.
        assert_eq!(
            read_frame(&mut frames).unwrap().unwrap(),
            (
                Msg::Write {
                    path: path("a.rs"),
                    mode: st.roots[&root].manifest[Path::new("a.rs")].mode
                },
                b"edited".to_vec()
            )
        );
        assert!(matches!(read_frame(&mut frames).unwrap().unwrap(),
            (Msg::Write { path: p, .. }, data) if p == path("c.rs") && data == b"oops"));
        assert!(matches!(
            read_frame(&mut frames).unwrap().unwrap().0,
            Msg::Delete { path: p } if p == path("b.rs")
        ));
        assert!(!st.roots[&root].manifest.contains_key(Path::new("b.rs")));

        // Another connection wrote under the root: nothing is known now.
        let displaced = SyncAck {
            lost: vec![name.clone()],
            ..Default::default()
        };
        // A build of another workspace still runs; the root is remirrored.
        let elsewhere = [PathBuf::from("/elsewhere/other-checkout")];
        assert!(check_sync_failures(&mut st, &displaced, Some(&elsewhere)).is_ok());
        assert!(st.needs_mirror(&root));
        assert!(
            check_sync_failures(&mut st, &displaced, Some(std::slice::from_ref(&root))).is_err()
        );
        assert!(check_sync_failures(&mut st, &displaced, None).is_err());
        assert!(st.needs_mirror(&root));
        assert!(st.roots[&root].mark.token.is_none());
        assert!(reverify(&d, &mut st, &link, &root).is_none());

        // An agent that does not confirm leaves no claim behind.
        st.roots.get_mut(&root).unwrap().mark.token = Some("stale".into());
        st.roots.get_mut(&root).unwrap().mark.hashes =
            snapshot_hashes(&root, &st.roots[&root].manifest);
        let agent = answer(false, frames);
        assert!(reverify(&d, &mut st, &link, &root).is_none());
        agent.join().unwrap();
        assert!(st.needs_mirror(&root));
        assert!(st.roots[&root].mark.token.is_none());
        link.kill();
        drop(st);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn source_sync_error_preserves_shared_link_and_unrelated_run_route() {
        let base = std::env::temp_dir().join(format!("jr-sync-isolation-{}", std::process::id()));
        std::fs::create_dir_all(&base).unwrap();
        let mut child = Command::new("cat")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdin = Arc::new(Mutex::new(BufWriter::new(child.stdin.take().unwrap())));
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let peer = peercache::Relay::new(true, |_, _| true);
        let link = Arc::new(Link {
            outage: Arc::default(),
            child: Mutex::new(child),
            stdin,
            routes: Mutex::default(),
            peer,
            checkpoint: false,
            // A guaranteed rsync receiver error without SSH or global env changes.
            mirror: Mirror {
                target: None,
                prefix: "/dev/null/".into(),
            },
        });
        let reader = link.clone();
        let (hint_tx, hint_rx) = std::sync::mpsc::sync_channel(1);
        let reader = std::thread::spawn(move || read_agent(&reader, stdout, Some(hint_tx)));
        let (run_tx, run_rx) = channel();
        link.routes.lock().unwrap().runs.insert(77, run_tx);
        let (watch_tx, watch_rx) = channel();
        let make_root = || Root {
            pending_repair: true,
            manifest: (0..=remote_sync::RSYNC_ABOVE)
                .map(|i| (PathBuf::from(format!("gone-{i}")), UNKNOWN))
                .collect(),
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        let other = base.join("unrelated");
        let d = Daemon {
            status_hint: None,
            sync: SyncQueue::new(Sync {
                install_retry: Default::default(),
                link_parts: None,
                recent_roots: Vec::new(),
                receipt: None,
                backend: Backend::Hosted,
                link: Some(link.clone()),
                roots: BTreeMap::from([(base.clone(), make_root()), (other.clone(), make_root())]),
            }),
            info: Mutex::default(),
            next_id: AtomicU64::new(1),
            watcher: Watcher::new(channel().0).unwrap(),
            sock: (base.join("unused.sock"), (0, 0)),
            lock: Mutex::new(None),
            draining: AtomicBool::new(false),
            peer: Arc::default(),
            peer_sock: Mutex::default(),
            hosted_agent: hosted_admit_agent(),
            pre_synced: Mutex::default(),
            live_link_ptr: AtomicUsize::new(0),
            warm_admit: Mutex::default(),
            leases: Arc::default(),
            relink: None,
        };
        watch_tx.send(base.clone()).unwrap();
        drop(watch_tx);
        push_loop(&d, watch_rx);
        {
            let st = d.sync.lock().unwrap();
            assert!(Arc::ptr_eq(st.link.as_ref().unwrap(), &link));
            assert!(st.needs_mirror(&base));
            assert!(!st.needs_mirror(&other));
        }
        assert!(link.alive());
        assert!(link.child.lock().unwrap().try_wait().unwrap().is_none());
        link.send(
            &Msg::Exit {
                id: 77,
                code: 0,
                timing: None,
            },
            &[],
        )
        .unwrap();
        let (msg, _) = run_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(
            msg,
            Msg::Exit {
                id: 77,
                code: 0,
                ..
            }
        ));
        link.kill();
        reader.join().unwrap();
        hint_rx.recv_timeout(Duration::from_secs(1)).unwrap();
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn failed_sync_invalidates_writes_and_deletes_without_touching_other_roots() {
        let stamp = remote_sync::Stamp {
            mtime_ns: 1,
            size: 12,
            mode: 0o644,
        };
        let root = PathBuf::from("/workspace");
        let other = PathBuf::from("/workspace-other");
        let entry = PathBuf::from("src/lib.rs");
        let make_root = || Root {
            pending_repair: false,
            manifest: Manifest::from([(entry.clone(), stamp)]),
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        let mut sync = Sync {
            install_retry: Default::default(),
            link_parts: None,
            recent_roots: Vec::new(),
            receipt: None,
            backend: Backend::Hosted,
            link: None,
            roots: BTreeMap::from([(root.clone(), make_root()), (other.clone(), make_root())]),
        };
        let both = [root.clone(), other.clone()];
        let before = source_receipt(&mut sync, &both).unwrap();
        // Independent SipHash-2-4 fixture for the existing length-prefixed v1 stream.
        #[cfg(target_endian = "little")]
        assert_eq!(before, "b021354f5cc84f2e61bbba7ade2452d8");
        assert_eq!(
            before,
            source_receipt(&mut sync, &[other.clone(), root.clone(), other.clone()]).unwrap()
        );
        let other_only = source_receipt(&mut sync, std::slice::from_ref(&other)).unwrap();
        assert_ne!(before, other_only);
        assert_eq!(before, source_receipt(&mut sync, &both).unwrap());
        assert!(check_sync_failures(&mut sync, &SyncAck::default(), None).is_ok());
        assert_eq!(sync.roots[&root].manifest[&entry], stamp);
        assert!(
            check_sync_failures(
                &mut sync,
                &SyncAck {
                    failed: vec![
                        "/workspace/src/lib.rs".into(),
                        "/workspace/deleted.rs".into()
                    ],
                    disk_full: false,
                    lost: Vec::new(),
                },
                None,
            )
            .is_err()
        );
        assert!(sync.receipt.is_none());
        assert_ne!(before, source_receipt(&mut sync, &both).unwrap());
        assert_eq!(
            other_only,
            source_receipt(&mut sync, std::slice::from_ref(&other)).unwrap()
        );
        assert_eq!(sync.roots[&root].manifest[&entry], UNKNOWN);
        assert_eq!(sync.roots[&root].manifest[Path::new("deleted.rs")], UNKNOWN);
        assert_eq!(sync.roots[&other].manifest[&entry], stamp);
        let generic = check_sync_failures(
            &mut sync,
            &SyncAck {
                failed: vec!["/workspace/src/lib.rs".into()],
                disk_full: false,
                lost: Vec::new(),
            },
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(!generic.contains("disk full"), "{generic}");
        let full = check_sync_failures(
            &mut sync,
            &SyncAck {
                failed: vec!["/workspace/src/lib.rs".into()],
                disk_full: true,
                lost: Vec::new(),
            },
            None,
        )
        .unwrap_err()
        .to_string();
        assert!(
            full.contains("hosted build machine disk full") && full.contains("No space left"),
            "{full}"
        );
    }

    #[test]
    fn reconnect_invalidates_all_live_roots_but_forgets_deleted_ones() {
        let base = std::env::temp_dir().join(format!("jr-reconnect-{}", std::process::id()));
        let a = base.join("a");
        let b = base.join("b");
        let gone = base.join("gone");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let make_root = || Root {
            pending_repair: false,
            manifest: Manifest::new(),
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };
        let mut sync = Sync {
            install_retry: Default::default(),
            link_parts: None,
            recent_roots: Vec::new(),
            receipt: None,
            backend: Backend::Hosted,
            link: None,
            roots: BTreeMap::from([
                (a.clone(), make_root()),
                (b.clone(), make_root()),
                (gone.clone(), make_root()),
            ]),
        };
        assert!(!sync.needs_mirror(&a));
        sync.invalidate_sources();
        assert!(sync.needs_mirror(&a));
        assert!(sync.needs_mirror(&b));
        assert!(!sync.roots.contains_key(&gone));
        assert!(sync.needs_mirror(&base.join("new")));
        // Resyncing one workspace must not accidentally mark another fresh.
        sync.roots.insert(a.clone(), make_root());
        assert!(!sync.needs_mirror(&a));
        assert!(sync.needs_mirror(&b));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn hosted_mirror_namespaces_isolate_late_full_sync_and_preserve_reconnect() {
        // Hosted mirrors live in a POSIX filesystem, even when the client runs
        // on Windows. Mixing `pwd -P` with a native C:\\... source path in the
        // local Mirror mock produces an invalid .../C:\\... filename. Keep the
        // simulated remote operations in the same shell/path domain as hosting.
        let old_namespace = remote_checkpoint::new_token();
        let current_namespace = remote_checkpoint::new_token();
        assert_ne!(old_namespace, current_namespace);
        assert_eq!(hosted_mirror_namespace(), hosted_mirror_namespace());
        let script = format!(
            r#"
set -eu
base=$(mktemp -d)
trap 'rm -rf "$base"' EXIT
HOME="$base/remote home"
export HOME
mkdir -p "$HOME" "$base/client/project/src"
old=$({old_script})
current=$({current_script})
test "$old" != "$current"
test "$current" = "$({current_script})"
source="$base/client/project"
live="$current$source"
retired="$old$source"
mkdir -p "$live" "$retired"
# Model a complete sync with deletion using tools also available in Git Bash.
# rsync itself is not installed on the Windows CI image. Replacing the whole
# destination is deliberately destructive, so namespace overlap cannot pass.
full_sync() {{
    rm -rf "$1"
    mkdir -p "$1"
    cp -R "$source/." "$1/"
}}

# The live daemon mirrors a new lib target and manifest.
printf "[package]\nname='project'\nversion='0.1.0'\n[lib]\npath='src/lib.rs'\n" > "$source/Cargo.toml"
printf 'pub fn current() {{}}\n' > "$source/src/lib.rs"
cp "$source/Cargo.toml" "$base/expected-manifest"
cp "$source/src/lib.rs" "$base/expected-lib"
full_sync "$live"

# A retired daemon sends an older full checkout, including --delete.
# Seed an obsolete file so the test proves that deletion really occurred.
printf 'obsolete\n' > "$retired/stale"
printf "[package]\nname='old'\n" > "$source/Cargo.toml"
rm "$source/src/lib.rs"
full_sync "$retired"
test ! -e "$retired/stale"
test ! -e "$retired/src/lib.rs"
cmp "$source/Cargo.toml" "$retired/Cargo.toml"
cmp "$base/expected-manifest" "$live/Cargo.toml"
cmp "$base/expected-lib" "$live/src/lib.rs"
"#,
            old_script = hosted_mirror_script(&old_namespace),
            current_script = hosted_mirror_script(&current_namespace),
        );
        let out = Command::new("sh").args(["-c", &script]).output().unwrap();
        assert!(
            out.status.success(),
            "hosted mirror simulation failed: {}\nstdout: {}\nstderr: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
    }

    #[cfg(unix)]
    #[test]
    fn hosted_mirror_namespaces_real_transfer_isolates_late_full_sync_and_preserves_reconnect() {
        let base = std::env::temp_dir().join(format!(
            "jr-hosted-source-isolation-{}",
            remote_checkpoint::new_token()
        ));
        let home = base.join("remote-home");
        let source = base.join("client/project");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(source.join("src")).unwrap();
        let resolve = |namespace: &str| {
            let out = Command::new("sh")
                .args(["-c", &hosted_mirror_script(namespace)])
                .env("HOME", &home)
                .output()
                .unwrap();
            assert!(out.status.success());
            String::from_utf8(out.stdout).unwrap().trim().to_string()
        };
        let old_namespace = remote_checkpoint::new_token();
        let current_namespace = remote_checkpoint::new_token();
        let old = Mirror {
            target: None,
            prefix: resolve(&old_namespace),
        };
        let current = Mirror {
            target: None,
            prefix: resolve(&current_namespace),
        };
        assert_ne!(old.prefix, current.prefix);
        assert_eq!(current.prefix, resolve(&current_namespace));
        assert_eq!(hosted_mirror_namespace(), hosted_mirror_namespace());

        // The live daemon mirrors a new lib target and manifest.
        let manifest = "[package]\nname='project'\nversion='0.1.0'\n[lib]\npath='src/lib.rs'\n";
        std::fs::write(source.join("Cargo.toml"), manifest).unwrap();
        std::fs::write(source.join("src/lib.rs"), "pub fn current() {}\n").unwrap();
        current.prepare(&source).unwrap();
        current.rsync(&source).unwrap();

        // A retired daemon now sends an older complete checkout, including
        // --delete. It must not remove the live lib or replace its manifest.
        std::fs::write(source.join("Cargo.toml"), "[package]\nname='old'\n").unwrap();
        std::fs::remove_file(source.join("src/lib.rs")).unwrap();
        old.prepare(&source).unwrap();
        old.rsync(&source).unwrap();
        let live = PathBuf::from(current.remote(&source));
        assert_eq!(
            std::fs::read_to_string(live.join("Cargo.toml")).unwrap(),
            manifest
        );
        assert_eq!(
            std::fs::read_to_string(live.join("src/lib.rs")).unwrap(),
            "pub fn current() {}\n"
        );
        assert!(
            !PathBuf::from(old.remote(&source))
                .join("src/lib.rs")
                .exists()
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn resolve_remote_prefix_uses_non_overlapping_mirror_dir_in_script() {
        // The fallback remote mirror path must not live inside ~/.justrust,
        // because remote_evict_registry::validate_prefix rejects prefixes that
        // overlap paths::root() (~/.justrust).
        assert!(
            REMOTE_MIRROR_PREFIX_SCRIPT.contains("~/.justrust-mirror"),
            "fallback remote mirror path should use ~/.justrust-mirror"
        );
        assert!(
            !REMOTE_MIRROR_PREFIX_SCRIPT.contains("~/.justrust/"),
            "fallback remote mirror path must not overlap ~/.justrust"
        );
    }

    #[test]
    fn only_another_version_or_a_confirmed_other_binary_costs_a_reinstall() {
        let unasked = || -> Option<bool> { panic!("an answer needs no second question") };
        // The agent answered in another protocol version.
        assert!(stamp_is_stale(Some(VERSION + 1), unasked));
        assert!(stamp_is_stale(Some(VERSION - 1), unasked));
        // No answer: a dropped link, EOF, a transport error.
        assert!(!stamp_is_stale(None, || None), "machine unreachable");
        assert!(!stamp_is_stale(None, || Some(true)), "binary is ours");
        assert!(stamp_is_stale(None, || Some(false)), "binary replaced");
        // Seven drops in a row over a bad link: no install.
        let installs = (0..7).filter(|_| stamp_is_stale(None, || None)).count();
        assert_eq!(installs, 0);
    }

    #[test]
    fn outage_counts_only_reattached_drops_and_flags_an_unrecovered_one() {
        let o = Outage::default();
        o.observe("justrust artifacts: unrelated");
        assert!(!o.is_down() && o.restored_ms() == 0.0);
        o.observe("hosted: transport interrupted, reattaching original session (1/3)");
        o.observe("hosted: transport interrupted, reattaching original session (2/3)");
        assert!(o.is_down());
        // The bridge measured the silence: its figure wins over our clock.
        o.observe("hosted: transport restored after 12.500s");
        assert!(!o.is_down());
        assert!((o.restored_ms() - 12_500.0).abs() < 1e-6);
        assert!((o.settled_ms() - 12_500.0).abs() < 1e-6);
        o.observe("hosted: transport restored after 0.250s");
        assert!((o.restored_ms() - 12_750.0).abs() < 1e-6);
        // A drop the bridge gave up on is what asks for a background relink.
        o.observe("hosted: transport interrupted, reattaching original session (1/3)");
        assert!(o.is_down());
        assert!((o.settled_ms() - 12_750.0).abs() < 1e-6);
    }

    #[test]
    fn warm_admit_requires_live_link_matching_key_and_remaining_quota() {
        let k1 = remote_checkpoint::digest(b"sk-key-one");
        let k2 = remote_checkpoint::digest(b"sk-key-two");
        let slot = Some(WarmAdmit {
            link_ptr: 42,
            key_hash: k1.clone(),
            at: Instant::now(),
        });
        let ready = |slot: &Option<WarmAdmit>, live: usize, key: &str| {
            live != 0
                && slot.as_ref().is_some_and(|w| {
                    w.link_ptr == live && w.key_hash == key && w.at.elapsed() < WARM_ADMIT_TTL
                })
        };
        assert!(ready(&slot, 42, &k1));
        assert!(!ready(&slot, 0, &k1), "disconnected link must not qualify");
        assert!(!ready(&slot, 99, &k1), "replaced link must not qualify");
        assert!(!ready(&slot, 42, &k2), "different API key must not qualify");

        assert!(warm_admit_body_ok(
            r#"{"admission":"maintenance-v1","update":null,"builds":{"can_build":true,"included":100,"included_used":10}}"#
        ));
        assert!(warm_admit_body_ok(
            r#"{"admission":"maintenance-v1","update":null,"builds":{"can_build":true,"included":10,"included_used":10,"overage_price_usd":0.01}}"#
        ));
        assert!(!warm_admit_body_ok(
            r#"{"admission":"maintenance-v1","update":null,"builds":{"can_build":true,"included":10,"included_used":9}}"#
        ));
        assert!(!warm_admit_body_ok(
            r#"{"admission":"maintenance-v1","update":null,"builds":{"can_build":false,"included":100,"included_used":0}}"#
        ));
        assert!(warm_admit_body_ok(
            r#"{"admission":"maintenance-v1","update":{"id":"u0","state":"completed"},"builds":{"can_build":true,"included":100,"included_used":10}}"#
        ));
        assert!(!warm_admit_body_ok(
            r#"{"admission":"maintenance-v1","update":{"id":"u1","state":"announced"},"builds":{"can_build":true,"included":100,"included_used":0}}"#
        ));
    }

    #[test]
    fn build_lease_checkout_tracks_quota_refill_exhaustion_and_safety_guards() {
        let k1 = remote_checkpoint::digest(b"sk-key-one");
        let k2 = remote_checkpoint::digest(b"sk-key-two");
        let now = Instant::now();
        let mut pool = LeasePool {
            active: Some(BuildLease {
                lease_id: "bl_test".into(),
                key: "sk-key-one".into(),
                key_hash: k1.clone(),
                link_ptr: 0,
                granted: 2,
                used: 0,
                expires_at: now + Duration::from_secs(120),
                builds: serde_json::json!({"can_build": true, "included": 100, "included_used": 10}),
                fleet_version: Some("0.1.36".into()),
                pending_runs: Vec::new(),
            }),
            ..LeasePool::default()
        };

        // Disconnected link or mismatched key must not check out.
        assert!(
            checkout_from_pool(&mut pool, &k1, 0, serde_json::json!({"run_id": "r0"}), now)
                .is_none()
        );
        assert!(
            checkout_from_pool(&mut pool, &k2, 42, serde_json::json!({"run_id": "r0"}), now)
                .is_none()
        );

        // First checkout binds link_ptr = 42, increments included_used to 11, and flags refill (remaining 1 <= 4).
        let (body1, need_refill1) = checkout_from_pool(
            &mut pool,
            &k1,
            42,
            serde_json::json!({"run_id": "r1", "cmd": "check"}),
            now,
        )
        .expect("first checkout");
        assert!(need_refill1);
        let v1: serde_json::Value = serde_json::from_str(&body1).unwrap();
        assert_eq!(v1["admission"], "maintenance-v1");
        assert_eq!(v1["lease_hit"], true);
        assert_eq!(v1["builds"]["included_used"], 11);
        assert_eq!(v1["fleet_version"], "0.1.36");
        assert_eq!(pool.active.as_ref().unwrap().link_ptr, 42);

        // Replaced link_ptr must be rejected once bound.
        assert!(
            checkout_from_pool(&mut pool, &k1, 99, serde_json::json!({"run_id": "r2"}), now)
                .is_none()
        );

        // Second checkout exhausts granted=2 and moves the lease to draining for close: true.
        let (body2, _) = checkout_from_pool(
            &mut pool,
            &k1,
            42,
            serde_json::json!({"run_id": "r2", "cmd": "test"}),
            now,
        )
        .expect("second checkout");
        let v2: serde_json::Value = serde_json::from_str(&body2).unwrap();
        assert_eq!(v2["builds"]["included_used"], 12);
        assert!(pool.active.is_none());
        assert_eq!(pool.draining.len(), 1);
        assert_eq!(pool.draining[0].pending_runs.len(), 2);

        // Near-expiry lease within safety margin must not check out.
        pool.active = Some(BuildLease {
            lease_id: "bl_expiring".into(),
            key: "sk-key-one".into(),
            key_hash: k1.clone(),
            link_ptr: 42,
            granted: 16,
            used: 0,
            expires_at: now + Duration::from_secs(5),
            builds: serde_json::json!({"can_build": true, "included": 100, "included_used": 12}),
            fleet_version: None,
            pending_runs: Vec::new(),
        });
        assert!(
            checkout_from_pool(&mut pool, &k1, 42, serde_json::json!({"run_id": "r3"}), now)
                .is_none()
        );

        // Eligibility checks: monthly included >=2 qualifies; trial/overage/maintenance do not.
        assert!(lease_eligible_body(
            r#"{"admission":"maintenance-v1","update":null,"builds":{"can_build":true,"included":100,"included_used":10}}"#
        ));
        assert!(!lease_eligible_body(
            r#"{"admission":"maintenance-v1","update":null,"builds":{"can_build":true,"included":10,"included_used":9,"overage_price_usd":0.01}}"#
        ));
        assert!(!lease_eligible_body(
            r#"{"admission":"maintenance-v1","update":null,"builds":{"can_build":true,"trial_remaining":50}}"#
        ));
        assert!(!lease_eligible_body(
            r#"{"admission":"maintenance-v1","update":{"id":"u1","state":"announced"},"builds":{"can_build":true,"included":100,"included_used":0}}"#
        ));
    }

    #[test]
    fn plan_dirty_paths_handles_edits_deletes_tempfiles_and_fallbacks() {
        let root = std::env::temp_dir().join(format!("jr-dirty-plan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        let lib = root.join("src/lib.rs");
        let gone = root.join("src/gone.rs");
        std::fs::write(&lib, "fn a() {}\n").unwrap();
        std::fs::write(&gone, "fn b() {}\n").unwrap();

        let scan = remote_sync::scan(&root).unwrap();
        let tracked = Root {
            pending_repair: false,
            manifest: scan.files,
            watch_dirs: None,
            oversized: Vec::new(),
            needs_full_sync: false,
            mark: Default::default(),
            clean_event_gen: None,
        };

        // Unchanged tracked file + vanished editor temp file -> empty plan (no scan).
        let plan = plan_dirty_paths(&root, &tracked, &[lib.clone(), root.join("src/lib.rs.tmp")])
            .expect("unchanged file and vanished temp file stay incremental");
        assert!(plan.changed.is_empty());
        assert!(plan.deleted.is_empty());

        // Edited tracked file + deleted tracked file -> exact incremental plan.
        std::fs::write(&lib, "fn a() { let _ = 1; }\n").unwrap();
        std::fs::remove_file(&gone).unwrap();
        let plan = plan_dirty_paths(
            &root,
            &tracked,
            &[lib.clone(), gone.clone(), root.join("src/lib.rs.tmp")],
        )
        .expect("edit + delete of tracked files stay incremental");
        assert_eq!(plan.changed.len(), 1);
        assert_eq!(plan.changed[0].0, PathBuf::from("src/lib.rs"));
        assert_eq!(plan.deleted, vec![PathBuf::from("src/gone.rs")]);

        // Brand-new persistent file falls back to full scan so gitignore rules run.
        let added = root.join("src/new.rs");
        std::fs::write(&added, "pub fn c() {}\n").unwrap();
        assert!(plan_dirty_paths(&root, &tracked, &[added]).is_none());

        // Ignore-rule file edit falls back to full scan.
        let gi = root.join(".gitignore");
        std::fs::write(&gi, "target/\n").unwrap();
        assert!(plan_dirty_paths(&root, &tracked, &[gi]).is_none());

        // A path the root's `.justrustignore` excludes is not its to mirror:
        // no plan entry and no full scan. Editing the ignore file rescans.
        let ji = root.join(".justrustignore");
        std::fs::write(&ji, "/vendor/\n").unwrap();
        std::fs::create_dir_all(root.join("vendor/tool/src")).unwrap();
        let vendored = root.join("vendor/tool/src/lib.rs");
        std::fs::write(&vendored, "pub fn v() {}\n").unwrap();
        let plan = plan_dirty_paths(&root, &tracked, std::slice::from_ref(&vendored))
            .expect("an excluded path stays incremental");
        assert!(plan.changed.is_empty());
        assert!(plan.deleted.is_empty());
        assert!(plan_dirty_paths(&root, &tracked, &[ji]).is_none());

        std::fs::remove_dir_all(&root).unwrap();
    }
}
