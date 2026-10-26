//! `justrust __agent`: the remote half of the sync daemon.
//!
//! Started by the local daemon over one long-lived ssh session. Reads frames
//! from stdin and writes frames to stdout:
//!
//! - `Write`/`Delete`/`Mkdir` are applied in order (atomic rename for
//!   writes), so a `Barrier` acknowledged after them means the tree matches.
//! - `Run` starts `justrust <args>` in `cwd` and streams its output back as
//!   `Out` frames, then `Exit` with the run's summary.json as payload.
//!   Several runs may be in flight (parallel agents).
//! - `Cancel` sends SIGINT to a run's process group.
//!
//! Paths arrive absolute. With a non-empty prefix (a machine where the local
//! paths cannot be created) every path is rebased under it.

use crate::sys::Detach;
use anyhow::{Context, Result, bail};
use std::collections::{BTreeSet, HashMap};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex};

use crate::remote_proto::{
    CACHE_VERSION, MIN_VERSION, Msg, RUN_FILES_VERSION, VERSION, read_frame, write_frame,
};

#[derive(Default)]
struct RunStatus {
    pid: Option<i32>,
    cancelled: bool,
}

type Out = Arc<Mutex<Box<dyn Write + Send>>>;

/// Process groups of runs in flight, for the signal handler: a killed
/// agent (`pkill`, a dropped session's SIGHUP) takes its builds with it
/// instead of leaving them to hold the cargo lock against the next agent.
static RUN_PGIDS: [AtomicI32; 64] = [const { AtomicI32::new(0) }; 64];

fn track_pgid(pid: i32) {
    for slot in &RUN_PGIDS {
        if slot
            .compare_exchange(0, pid, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            return;
        }
    }
}

fn untrack_pgid(pid: i32) {
    for slot in &RUN_PGIDS {
        let _ = slot.compare_exchange(pid, 0, Ordering::SeqCst, Ordering::SeqCst);
    }
}

extern "C" fn on_fatal_signal(sig: libc::c_int) {
    for slot in &RUN_PGIDS {
        let pid = slot.load(Ordering::SeqCst);
        if pid > 0 {
            // SAFETY: kill is async-signal-safe.
            unsafe {
                libc::kill(-pid, libc::SIGTERM);
            }
        }
    }
    // SAFETY: _exit is async-signal-safe.
    unsafe { libc::_exit(128 + sig) }
}

fn install_signal_handlers() {
    for sig in [libc::SIGTERM, libc::SIGHUP, libc::SIGINT, libc::SIGPIPE] {
        // SAFETY: the handler only calls async-signal-safe functions.
        unsafe {
            libc::signal(sig, on_fatal_signal as *const () as libc::sighandler_t);
        }
    }
}

fn send(out: &Out, m: &Msg, payload: &[u8]) {
    if let Ok(mut w) = out.lock() {
        let _ = write_frame(&mut *w, m, payload);
    }
}

fn write_atomic(path: &Path, data: &[u8], mode: u32) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or(Path::new("/"));
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(
        ".{}.jr-tmp",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    let res = std::fs::write(&tmp, data)
        .and_then(|()| crate::sys::set_mode(&tmp, mode & 0o777))
        .and_then(|()| std::fs::rename(&tmp, path));
    if res.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    res
}

/// One low-disk collection pass over this agent's mirrored roots
/// (`JUSTRUST_REMOTE_EVICT`: on by default). Logged to the agent's stderr.
fn evict_now(roots: &crate::remote_evict::Shared) {
    use crate::remote_evict as ev;
    let Some(home) = dirs::home_dir() else { return };
    let Ok(registry) = roots.lock() else { return };
    ev::collect(
        &registry,
        &home,
        ev::Mode::from_env(),
        ev::DEFAULT_GRACE,
        &ev::System,
        &mut |line| eprintln!("justrust agent: {line}"),
    );
}

/// Free bytes if below the build floor even after an eviction attempt.
fn low_disk_after_eviction(roots: &crate::remote_evict::Shared) -> Option<u64> {
    use crate::remote_evict as ev;
    let home = dirs::home_dir()?;
    ev::below_floor(&ev::System, &home)?;
    evict_now(roots);
    ev::below_floor(&ev::System, &home)
}

/// ENOSPC ("No space left on device") or a quota exhaustion.
pub(crate) fn is_disk_full(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::StorageFull | std::io::ErrorKind::QuotaExceeded
    ) || matches!(e.raw_os_error(), Some(libc::ENOSPC | libc::EDQUOT))
}

fn remove(path: &Path) -> std::io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
        Ok(m) if m.is_dir() => std::fs::remove_dir_all(path),
        Ok(_) => std::fs::remove_file(path),
    }
}

pub fn main(prefix: Option<String>) -> ! {
    install_signal_handlers();
    let out: Box<dyn Write + Send> = Box::new(BufWriter::new(std::io::stdout()));
    let code = match serve(std::io::stdin().lock(), out, prefix.unwrap_or_default()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("justrust agent: {e:#}");
            1
        }
    };
    std::process::exit(code)
}

/// Runs need a real, immutable path: an atomic agent upgrade unlinks the
/// original executable, and re-execing /proc/self/exe alone still leaves a
/// child whose current_exe cannot be canonicalized by the recording shims.
/// Identify the open inode, publish a copy without replacing any existing
/// snapshot, and retain it for all connections and children using that build.
fn executable_snapshot() -> Result<PathBuf> {
    use std::os::unix::fs::MetadataExt;
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let mut source = crate::procfs::running_executable()?;
    let meta = source.metadata()?;
    let mut hash = crate::depcache::H128::new();
    for field in [
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime() as u64,
        meta.mtime_nsec() as u64,
    ] {
        hash.field(&field.to_le_bytes());
    }
    let dir = crate::paths::home()?.join("bin/agent").join(hash.hex());
    let snapshot = dir.join("justrust");
    if crate::sys::is_executable(&snapshot) && std::fs::metadata(&snapshot)?.len() == meta.len() {
        return Ok(snapshot);
    }
    std::fs::create_dir_all(&dir)?;
    let tmp = dir.join(format!(
        ".snapshot-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| -> Result<()> {
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        std::io::copy(&mut source, &mut output)?;
        crate::sys::set_mode(&tmp, 0o755)?;
        // A rename would unlink an already-running concurrent snapshot. A
        // same-directory hard link installs atomically without replacement.
        match std::fs::hard_link(&tmp, &snapshot) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                if crate::sys::is_executable(&snapshot)
                    && std::fs::metadata(&snapshot)?.len() == meta.len()
                {
                    Ok(())
                } else {
                    bail!("invalid agent executable snapshot {}", snapshot.display())
                }
            }
            Err(e) => Err(e.into()),
        }
    })();
    let _ = std::fs::remove_file(&tmp);
    result?;
    Ok(snapshot)
}

/// Serve one daemon connection until it closes. Builds still running when
/// it does are interrupted: nobody is left to read their output, and they
/// would hold the cargo lock against the next connection's builds.
pub fn serve<R: Read>(mut input: R, out: Box<dyn Write + Send>, prefix: String) -> Result<()> {
    let out: Out = Arc::new(Mutex::new(out));
    // Older daemons still speak v1, a subset of v2 (they never send
    // `Synced` and ignore the new fields): serve them in their version so a
    // machine shared by two justrust builds keeps working for both.
    let version = match read_frame(&mut input)? {
        Some((Msg::Hello { version }, _)) if (MIN_VERSION..=VERSION).contains(&version) => version,
        Some((Msg::Hello { version }, _)) => {
            send(
                &out,
                &Msg::Error {
                    id: 0,
                    msg: format!("protocol {version}, agent speaks {MIN_VERSION} to {VERSION}"),
                },
                &[],
            );
            bail!("protocol mismatch");
        }
        _ => bail!("expected Hello"),
    };
    let executable = executable_snapshot()?;
    send(&out, &Msg::Hello { version }, &[]);
    // Peer dependency cache: this machine's cache serves the daemon's
    // requests, and this machine's rustc shims reach the daemon's machine
    // through `peer.sock`.
    let relay = {
        let out = out.clone();
        crate::peercache::Relay::new(version >= CACHE_VERSION, move |m, p| {
            out.lock()
                .is_ok_and(|mut w| write_frame(&mut *w, m, p).is_ok())
        })
    };
    if relay.capable
        && let Some(p) = crate::peercache::sock_path()
    {
        let current: crate::peercache::Current = Arc::new(Mutex::new(Some(relay.clone())));
        // No key prefetch: the daemon may be idle. The first shim query
        // loads the key set.
        if let Err(e) = crate::peercache::listen(&p, current) {
            eprintln!("justrust agent: peer cache socket {}: {e}", p.display());
        }
    }
    let at = |p: &str| -> PathBuf {
        if prefix.is_empty() {
            PathBuf::from(p)
        } else {
            Path::new(&prefix).join(p.trim_start_matches('/'))
        }
    };
    let pids: Arc<Mutex<HashMap<u64, RunStatus>>> = Arc::default();
    // Mirrored roots and their builds, for low-disk `target/` eviction.
    // An explicitly prefixed agent owns this mirror namespace. Create its
    // directory before opening the canonical durable scope, including first boot.
    if !prefix.is_empty() {
        std::fs::create_dir_all(&prefix)?;
    }
    let registry = crate::remote_evict::Registry::open(&prefix).unwrap_or_else(|e| {
        eprintln!("justrust agent: target GC disabled: cannot load private registry: {e}");
        crate::remote_evict::Registry::disabled(e.to_string())
    });
    let roots: crate::remote_evict::Shared = Arc::new(Mutex::new(registry));
    // Paths whose latest Write or Delete failed: the tree differs from what
    // the daemon believes. Reported with every barrier until a later frame
    // for the same path succeeds, so no build runs on a stale file.
    let mut failed: BTreeSet<String> = BTreeSet::new();
    // The subset of `failed` whose cause was a full disk (ENOSPC), so the
    // client can say "hosted disk full" instead of a generic sync failure.
    let mut disk_full: BTreeSet<String> = BTreeSet::new();
    let res = loop {
        let (msg, payload) = match read_frame(&mut input) {
            Ok(Some(f)) => f,
            Ok(None) => break Ok(()),
            Err(e) => break Err(e),
        };
        if relay.capable && relay.on_frame(&msg, &payload) {
            continue;
        }
        let sync_run = matches!(&msg, Msg::RunSync { .. });
        match msg {
            Msg::Write { path, mode } => match write_atomic(&at(&path), &payload, mode) {
                Ok(()) => {
                    failed.remove(&path);
                    disk_full.remove(&path);
                }
                Err(e) => {
                    eprintln!("justrust agent: write {path}: {e}");
                    if is_disk_full(&e) {
                        evict_now(&roots);
                        disk_full.insert(path.clone());
                    } else {
                        disk_full.remove(&path);
                    }
                    failed.insert(path);
                }
            },
            Msg::Delete { path } => match remove(&at(&path)) {
                Ok(()) => {
                    failed.remove(&path);
                    disk_full.remove(&path);
                }
                Err(e) => {
                    eprintln!("justrust agent: delete {path}: {e}");
                    if is_disk_full(&e) {
                        disk_full.insert(path.clone());
                    } else {
                        disk_full.remove(&path);
                    }
                    failed.insert(path);
                }
            },
            Msg::Mkdir { path } => {
                let _ = std::fs::create_dir_all(at(&path));
            }
            Msg::Synced { path } => {
                if let Ok(mut r) = roots.lock()
                    && let Err(e) = r.register(&at(&path))
                {
                    eprintln!("justrust agent: target GC enrollment refused for {path}: {e}");
                }
                let under = format!("{}/", path.trim_end_matches('/'));
                failed.retain(|p| !p.starts_with(&under));
                disk_full.retain(|p| !p.starts_with(&under));
            }
            Msg::Barrier { id } => send(
                &out,
                &Msg::BarrierAck {
                    id,
                    failed: failed.iter().cloned().collect(),
                    disk_full: !disk_full.is_empty(),
                },
                &[],
            ),
            Msg::Run {
                id,
                cwd,
                args,
                env,
                run_id,
                ..
            }
            | Msg::RunSync {
                id,
                cwd,
                args,
                env,
                run_id,
                ..
            } => {
                if sync_run {
                    send(
                        &out,
                        &Msg::BarrierAck {
                            id,
                            failed: failed.iter().cloned().collect(),
                            disk_full: !disk_full.is_empty(),
                        },
                        &[],
                    );
                    if !failed.is_empty() {
                        send(
                            &out,
                            &Msg::Error {
                                id,
                                msg: if disk_full.is_empty() {
                                    "source sync failed, build not started".into()
                                } else {
                                    "hosted build machine disk full: source sync failed, build not started".into()
                                },
                            },
                            &[],
                        );
                        continue;
                    }
                }
                let out = out.clone();
                let pids = pids.clone();
                let cwd = at(&cwd);
                let executable = executable.clone();
                // Register before spawning the worker: an immediate Cancel
                // must survive the window before its process has a PID.
                pids.lock().unwrap().insert(id, RunStatus::default());
                let roots = roots.clone();
                std::thread::spawn(move || {
                    let root = roots.lock().ok().and_then(|r| r.root_of(&cwd));
                    if let Some(free) = low_disk_after_eviction(&roots) {
                        pids.lock().unwrap().remove(&id);
                        send(
                            &out,
                            &Msg::Error {
                                id,
                                msg: format!(
                                    "hosted build machine disk full: {:.1} GiB free, build not started",
                                    free as f64 / (1u64 << 30) as f64
                                ),
                            },
                            &[],
                        );
                        return;
                    }
                    // Shared per-root lock: eviction of this root's target/
                    // waits for the build, and the build for an eviction.
                    let setup = (|| -> anyhow::Result<_> {
                        let Some(root) = &root else {
                            return Ok(None);
                        };
                        let lock = crate::remote_evict::build_lock(root)
                            .context("cannot acquire source-root build lock")?;
                        roots
                            .lock()
                            .map_err(|_| anyhow::anyhow!("activity registry poisoned"))?
                            .begin(root)?;
                        Ok(Some(lock))
                    })();
                    let _build_lock = match setup {
                        Ok(lock) => lock,
                        Err(e) => {
                            pids.lock().unwrap().remove(&id);
                            send(
                                &out,
                                &Msg::Error {
                                    id,
                                    msg: format!(
                                        "build not started: cannot protect source activity: {e:#}"
                                    ),
                                },
                                &[],
                            );
                            return;
                        }
                    };
                    let result =
                        start_run(&executable, id, &cwd, &args, &env, &run_id, &out, &pids);
                    if let (Some(root), Ok(mut r)) = (&root, roots.lock())
                        && let Err(e) = r.end(
                            root,
                            crate::remote_evict::Probe::now(&crate::remote_evict::System),
                        )
                    {
                        eprintln!(
                            "justrust agent: activity end failed for {} (GC stays protected): {e}",
                            root.display()
                        );
                    }
                    drop(_build_lock);
                    if let (Some(root), Ok(r)) = (&root, roots.lock())
                        && let Err(e) = r.finish_idle(root)
                    {
                        eprintln!(
                            "justrust agent: activity finalization failed for {} (GC stays protected): {e}",
                            root.display()
                        );
                    }
                    evict_now(&roots);
                    let code = match result {
                        Ok(c) => c,
                        Err(e) => {
                            pids.lock().unwrap().remove(&id);
                            send(
                                &out,
                                &Msg::Error {
                                    id,
                                    msg: format!("{e:#}"),
                                },
                                &[],
                            );
                            return;
                        }
                    };
                    let summary = summary_for(&run_id).unwrap_or_default();
                    if version >= RUN_FILES_VERSION
                        && let Some(dir) = run_dir(&run_id)
                    {
                        let files = crate::remote_proto::pack_run_files(&dir);
                        if !files.is_empty() {
                            send(&out, &Msg::RunFiles { id }, &files);
                        }
                    }
                    send(&out, &Msg::Exit { id, code }, &summary);
                });
            }
            Msg::Cancel { id } => {
                if let Ok(mut pids) = pids.lock()
                    && let Some(run) = pids.get_mut(&id)
                {
                    run.cancelled = true;
                    if let Some(pid) = run.pid {
                        // SAFETY: signalling our own child's process group.
                        unsafe {
                            libc::kill(-pid, libc::SIGINT);
                        }
                    }
                }
            }
            Msg::Ping => send(
                &out,
                &Msg::Pong {
                    roots: Vec::new(),
                    machine: String::new(),
                    pushed: 0,
                    connected: true,
                    build: String::new(),
                    active: pids.lock().map(|p| p.len() as u64).unwrap_or(0),
                    skipped: 0,
                },
                &[],
            ),
            _ => {}
        }
    };
    relay.close();
    if let Ok(mut p) = pids.lock() {
        for run in p.values_mut() {
            run.cancelled = true;
            if let Some(pid) = run.pid {
                // SAFETY: signalling our own child's process group.
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
        }
    }
    res
}

/// Workspaces cap `build.jobs` for their contributors' laptops (Jcode sets
/// `jobs = 4` for ~15 GB machines), which left 28 of the build machine's 32
/// cores idle. This machine runs only justrust builds, so size jobs for it:
/// every core, bounded by memory at ~2 GiB per job. Job count is not part
/// of any fingerprint. An explicit `-j`/`--jobs` or a client-sent
/// `CARGO_BUILD_JOBS` wins.
fn machine_jobs(args: &[String], env: &[(String, String)]) -> Option<usize> {
    if env.iter().any(|(k, _)| k == "CARGO_BUILD_JOBS")
        || args
            .iter()
            .take_while(|a| *a != "--")
            .any(|a| a == "-j" || a == "--jobs" || a.starts_with("--jobs=") || a.starts_with("-j"))
    {
        return None;
    }
    let cpus = std::thread::available_parallelism().ok()?.get();
    let mem_gib = crate::procfs::meminfo()
        .filter(|m| m.total_mb > 0)
        .map(|m| (m.total_mb >> 10) as usize);
    Some(jobs_for(cpus, mem_gib))
}

/// The toolchain a build on this machine uses when the project pins none:
/// the one the prebuilt store is compiled with
/// ([`crate::depcache_prebuilt::STORE_TOOLCHAIN`]). Keys include rustc
/// `-vV`, so a machine building with its system stable rustc could never
/// hit the store (0 prebuilt hits in every hosted run before, FINDINGS
/// 55). `None` (leave the choice alone) when the client sent
/// `JUSTRUST_TOOLCHAIN`, the project pins one (`justrust.toml`, or a
/// `rust-toolchain` file rustup would honor), or
/// `JUSTRUST_REMOTE_TOOLCHAIN` says otherwise (`system` keeps the machine's
/// rustc, any other value is used as the default spec).
fn store_toolchain(cwd: &Path, env: &[(String, String)]) -> Option<String> {
    if env
        .iter()
        .any(|(k, _)| matches!(k.as_str(), "JUSTRUST_TOOLCHAIN" | "RUSTUP_TOOLCHAIN"))
    {
        return None;
    }
    let want = std::env::var("JUSTRUST_REMOTE_TOOLCHAIN")
        .unwrap_or_else(|_| crate::depcache_prebuilt::STORE_TOOLCHAIN.to_owned());
    if matches!(want.trim(), "" | "system" | "off" | "0" | "none") {
        return None;
    }
    let pinned = cwd.ancestors().any(|d| {
        d.join(crate::toolchain::CONFIG_FILE).is_file()
            || d.join("rust-toolchain.toml").is_file()
            || d.join("rust-toolchain").is_file()
    });
    (!pinned).then_some(want)
}

fn jobs_for(cpus: usize, mem_gib: Option<usize>) -> usize {
    mem_gib.map_or(cpus, |m| cpus.min(m / 2)).max(1)
}

fn run_dir(run_id: &str) -> Option<PathBuf> {
    if run_id.is_empty() || run_id.contains(['/', '.']) {
        return None;
    }
    Some(crate::paths::runs_dir().ok()?.join(run_id))
}

fn summary_for(run_id: &str) -> Option<Vec<u8>> {
    std::fs::read(run_dir(run_id)?.join("summary.json")).ok()
}

/// Env var a client sends with a run: its cargo home.
pub const CLIENT_CARGO_HOME: &str = "JUSTRUST_CLIENT_CARGO_HOME";

/// Registry sources are compiled into crate metadata (and so into every
/// dependent's SVH) by absolute path. Building with the cargo home at the
/// client's path makes this machine's dependency artifacts interchangeable
/// with the client's (the peer cache, `src/peercache.rs`). The client's
/// path becomes a directory whose `registry` and `git` link to this
/// machine's own cargo home, so downloads are still shared. `None` (build
/// with the usual cargo home) when the path cannot be set up or the peer
/// cache is off.
fn mirror_cargo_home(client: &Path) -> Option<PathBuf> {
    if !crate::peercache::enabled() || !client.is_absolute() {
        return None;
    }
    let own = crate::depcache::cargo_home();
    if own.as_os_str().is_empty() || own == client {
        return None;
    }
    // Usable: each of `registry` and `git` links to our own, or is a real
    // directory (a cargo home someone already uses at that path: it works,
    // it only downloads separately).
    let ready = |p: &Path| {
        ["registry", "git"].iter().all(|d| {
            std::fs::read_link(p.join(d)).ok() == Some(own.join(d))
                || std::fs::symlink_metadata(p.join(d)).is_ok_and(|m| m.is_dir())
        }) && std::fs::metadata(p).is_ok_and(|m| !m.permissions().readonly())
    };
    if ready(client) {
        return Some(client.to_path_buf());
    }
    let s = client.to_str()?;
    let made = std::fs::create_dir_all(client).is_ok()
        || Command::new("sh")
            .arg("-c")
            .arg(crate::remote_sync::prepare_script(s))
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|st| st.success());
    if !made {
        return None;
    }
    for d in ["registry", "git"] {
        let target = own.join(d);
        let _ = std::fs::create_dir_all(&target);
        let link = client.join(d);
        if std::fs::read_link(&link).ok() != Some(target.clone()) {
            // A real directory there is a cargo home already in use: keep it.
            if std::fs::symlink_metadata(&link).is_ok_and(|m| m.is_dir()) {
                continue;
            }
            let _ = std::fs::remove_file(&link);
            std::os::unix::fs::symlink(&target, &link).ok()?;
        }
    }
    // Config and credentials apply as before.
    for f in ["config.toml", "config", "credentials.toml"] {
        let src = own.join(f);
        let link = client.join(f);
        if src.exists() && std::fs::symlink_metadata(&link).is_err() {
            let _ = std::os::unix::fs::symlink(&src, &link);
        }
    }
    ready(client).then(|| client.to_path_buf())
}

#[allow(clippy::too_many_arguments)]
fn start_run(
    executable: &Path,
    id: u64,
    cwd: &Path,
    args: &[String],
    env: &[(String, String)],
    run_id: &str,
    out: &Out,
    pids: &Arc<Mutex<HashMap<u64, RunStatus>>>,
) -> Result<i32> {
    let mut c = Command::new(executable);
    c.args(args)
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k, v)).filter(|(k, _)| {
            *k != CLIENT_CARGO_HOME
                && *k != crate::remote_proto::CLIENT_UMASK
                && *k != crate::remote_proto::CLIENT_LIVE_CPU
        }))
        .env_remove(crate::remote_proto::CLIENT_UMASK)
        .env_remove(crate::remote_proto::CLIENT_LIVE_CPU)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .detached();
    if let Some((_, value)) = env
        .iter()
        .find(|(k, _)| k == crate::remote_proto::CLIENT_UMASK)
    {
        let mask =
            crate::procfs::parse_umask(value).context("invalid caller file creation mask")?;
        use std::os::unix::process::CommandExt;
        // SAFETY: this runs only after fork in this child, before exec. umask
        // is async-signal-safe; no allocation, locking or parent-global change.
        unsafe {
            c.pre_exec(move || {
                libc::umask(mask as libc::mode_t);
                Ok(())
            });
        }
    }
    if !run_id.is_empty() {
        c.env("JUSTRUST_RUN_ID_OVERRIDE", run_id);
    }
    // Never route a build the agent runs back to another machine.
    c.env(crate::route::ON_AGENT, "1");
    if let Some(tc) = store_toolchain(cwd, env) {
        c.env("JUSTRUST_TOOLCHAIN", tc);
    }
    if let Some(j) = machine_jobs(args, env) {
        c.env("CARGO_BUILD_JOBS", j.to_string());
    }
    // Fetch every store hit here. The per-fetch overhead the cost model
    // assumes (700 ms, a laptop under agent load) declined 487 of 529
    // available hits on a 32-job build machine, where fetches overlap:
    // cold `check -p jcode-tui` 37.5-41.3 s costed vs 35.7-38.7 s fetching
    // all (FINDINGS 65). A client-sent value wins.
    if !env.iter().any(|(k, _)| k == "JUSTRUST_PREBUILT_ALWAYS") {
        c.env("JUSTRUST_PREBUILT_ALWAYS", "1");
    }
    // cargo and rustup live in ~/.cargo/bin, which a non-login ssh shell
    // may not have on PATH.
    if let Some(home) = dirs::home_dir() {
        let path = std::env::var("PATH").unwrap_or_default();
        c.env("PATH", format!("{}/.cargo/bin:{path}", home.display()));
    }
    if let Some((_, home)) = env.iter().find(|(k, _)| k == CLIENT_CARGO_HOME)
        && let Some(home) = mirror_cargo_home(Path::new(home))
    {
        c.env("CARGO_HOME", home);
    }
    // Hold the state lock only across spawn and PID publication. A Cancel
    // either prevents spawning or observes the exact child to interrupt.
    let mut states = pids.lock().unwrap();
    let state = states.get_mut(&id).context("run no longer active")?;
    if state.cancelled {
        bail!("run cancelled before starting");
    }
    let mut child = c.spawn()?;
    state.pid = Some(child.id() as i32);
    track_pgid(child.id() as i32);
    drop(states);
    let cpu_done: Arc<(Mutex<bool>, std::sync::Condvar)> = Arc::default();
    let cpu_pump = if env
        .iter()
        .any(|(k, v)| k == crate::remote_proto::CLIENT_LIVE_CPU && v == "1")
    {
        run_dir(run_id).map(|dir| {
            let out = out.clone();
            let done = cpu_done.clone();
            std::thread::spawn(move || {
                let mut previous = Vec::new();
                loop {
                    let guard = done.0.lock().unwrap();
                    if *guard {
                        break;
                    }
                    drop(guard);
                    if let Ok(file) = std::fs::File::open(dir.join("cpu.json")) {
                        let mut bytes = Vec::new();
                        if file.take(16 * 1024).read_to_end(&mut bytes).is_ok()
                            && bytes != previous
                            && serde_json::from_slice::<crate::record::LiveCpu>(&bytes).is_ok()
                        {
                            send(
                                &out,
                                &Msg::Out {
                                    id,
                                    stream: crate::remote_proto::LIVE_CPU_STREAM,
                                },
                                &bytes,
                            );
                            previous = bytes;
                        }
                    }
                    let guard = done.0.lock().unwrap();
                    let _ = done.1.wait_timeout_while(
                        guard,
                        std::time::Duration::from_millis(200),
                        |finished| !*finished,
                    );
                }
            })
        })
    } else {
        None
    };
    let mut pumps = Vec::new();
    for (stream, pipe) in [
        (
            1u8,
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
        (
            2u8,
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
    ] {
        let Some(mut pipe) = pipe else { continue };
        let out = out.clone();
        pumps.push(std::thread::spawn(move || {
            let mut buf = vec![0u8; 64 << 10];
            while let Ok(n) = pipe.read(&mut buf) {
                if n == 0 {
                    break;
                }
                send(&out, &Msg::Out { id, stream }, &buf[..n]);
            }
        }));
    }
    for p in pumps {
        let _ = p.join();
    }
    *cpu_done.0.lock().unwrap() = true;
    cpu_done.1.notify_all();
    if let Some(pump) = cpu_pump {
        let _ = pump.join();
    }
    let st = child.wait()?;
    pids.lock().unwrap().remove(&id);
    untrack_pgid(child.id() as i32);
    use std::os::unix::process::ExitStatusExt;
    Ok(st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_full_errors_are_classified() {
        assert!(is_disk_full(&std::io::Error::from_raw_os_error(
            libc::ENOSPC
        )));
        assert!(is_disk_full(&std::io::Error::from_raw_os_error(
            libc::EDQUOT
        )));
        assert!(is_disk_full(&std::io::Error::from(
            std::io::ErrorKind::StorageFull
        )));
        assert!(!is_disk_full(&std::io::Error::from_raw_os_error(
            libc::EACCES
        )));
        assert!(!is_disk_full(&std::io::Error::other("other")));
    }
    #[test]
    fn unpinned_projects_build_with_the_store_toolchain() {
        let d = std::env::temp_dir().join(format!("jr-store-tc-{}", std::process::id()));
        let sub = d.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        let want = Some(crate::depcache_prebuilt::STORE_TOOLCHAIN.to_owned());
        assert_eq!(store_toolchain(&sub, &[]), want);
        // The client chose one: keep it.
        let env = [("JUSTRUST_TOOLCHAIN".to_string(), "system".to_string())];
        assert_eq!(store_toolchain(&sub, &env), None);
        let env = [("RUSTUP_TOOLCHAIN".to_string(), "1.90.0".to_string())];
        assert_eq!(store_toolchain(&sub, &env), None);
        // The project pins one (rustup file or justrust.toml): keep it.
        std::fs::write(d.join("a/rust-toolchain.toml"), "").unwrap();
        assert_eq!(store_toolchain(&sub, &[]), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn jobs_use_the_whole_machine_within_memory() {
        assert_eq!(jobs_for(32, Some(61)), 30);
        assert_eq!(jobs_for(32, Some(124)), 32);
        assert_eq!(jobs_for(8, None), 8);
        assert_eq!(jobs_for(4, Some(1)), 1);
    }

    #[test]
    fn explicit_jobs_win() {
        let s = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(machine_jobs(&s(&["test", "-j", "2"]), &[]).is_none());
        assert!(machine_jobs(&s(&["test", "-j2"]), &[]).is_none());
        assert!(machine_jobs(&s(&["test", "--jobs=2"]), &[]).is_none());
        let env = [("CARGO_BUILD_JOBS".to_string(), "3".to_string())];
        assert!(machine_jobs(&s(&["test"]), &env).is_none());
        // Test-binary args after `--` are not cargo's.
        assert!(machine_jobs(&s(&["test", "--", "-j"]), &[]).is_some());
    }
}
