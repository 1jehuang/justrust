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

#[path = "remote_admission_gc.rs"]
mod admission_gc;

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
    if ev::Mode::from_env() == ev::Mode::On {
        ev::sweep_sent(false, &mut |line| eprintln!("justrust agent: {line}"));
    }
    ev::collect_shared(
        roots,
        &home,
        ev::Mode::from_env(),
        ev::DEFAULT_GRACE,
        &ev::System,
        &mut |line| eprintln!("justrust agent: {line}"),
    );
}

/// Opportunistic collection is maintenance, not part of every Cargo result.
/// Claim before scanning so concurrent runs share one bounded cadence. Urgent
/// low-disk admission and ENOSPC recovery deliberately bypass this limiter.
fn claim_periodic_collection(
    last: &Mutex<Option<std::time::Instant>>,
    now: std::time::Instant,
    interval: std::time::Duration,
) -> bool {
    let Ok(mut last) = last.lock() else {
        return false;
    };
    if last.is_some_and(|previous| now.saturating_duration_since(previous) < interval) {
        return false;
    }
    *last = Some(now);
    true
}

fn collection_interval(env: &[(String, String)]) -> std::time::Duration {
    // A sweep after every result can block the next build's registry access.
    // The low-disk floor and ENOSPC recovery bypass this interval.
    let secs = env
        .iter()
        .find(|(key, _)| key == "JUSTRUST_REMOTE_EVICT_INTERVAL_SECS")
        .and_then(|(_, value)| value.parse::<u64>().ok())
        .filter(|secs| *secs <= 3600)
        .unwrap_or(60);
    std::time::Duration::from_secs(secs)
}

/// Free bytes if below the build floor even after an eviction attempt.
fn low_disk_after_eviction(
    roots: &crate::remote_evict::Shared,
    gc: &admission_gc::AdmissionGc,
) -> Option<u64> {
    use crate::remote_evict as ev;
    let home = dirs::home_dir()?;
    gc.below_after(|| ev::below_floor(&ev::System, &home), || evict_now(roots))
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
        Some((Msg::Hello { version, .. }, _)) if (MIN_VERSION..=VERSION).contains(&version) => {
            version
        }
        Some((Msg::Hello { version, .. }, _)) => {
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
    send(
        &out,
        &Msg::Hello {
            version,
            caps: vec![crate::remote_checkpoint::CAP.into()],
        },
        &[],
    );
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
    // On graceful EOF drain only workers which have finished executing and are
    // publishing their result or clearing their durable activity marker.
    let publishing = Arc::new(std::sync::RwLock::new(()));
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
    let admission_gc = Arc::new(admission_gc::AdmissionGc::default());
    let last_periodic_collection = Arc::new(Mutex::new(None));
    // Paths whose latest Write or Delete failed: the tree differs from what
    // the daemon believes. Reported with every barrier until a later frame
    // for the same path succeeds, so no build runs on a stale file.
    let mut failed: BTreeSet<String> = BTreeSet::new();
    // The subset of `failed` whose cause was a full disk (ENOSPC), so the
    // client can say "hosted disk full" instead of a generic sync failure.
    let mut disk_full: BTreeSet<String> = BTreeSet::new();
    // Source checkpoints this connection wrote or verified
    // (`remote_checkpoint`): only its own frames may change those roots.
    let mut held: HashMap<PathBuf, crate::remote_checkpoint::Held> = HashMap::new();
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
        // Saved mirror state of the root no longer describes the tree.
        if let Msg::Write { path, .. } | Msg::Delete { path } = &msg {
            crate::remote_checkpoint::invalidate_above(&at(path), &held);
        }
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
                if let Err(e) = crate::remote_evict::Registry::enroll(&roots, &at(&path)) {
                    eprintln!("justrust agent: target GC enrollment refused for {path}: {e}");
                }
                let under = format!("{}/", path.trim_end_matches('/'));
                failed.retain(|p| !p.starts_with(&under));
                disk_full.retain(|p| !p.starts_with(&under));
                // Mirrored out of band: no checkpoint describes it now.
                held.remove(&at(&path));
                crate::remote_checkpoint::forget(&at(&path));
            }
            Msg::Checkpoint { path, token } => {
                let dir = at(&path);
                let under = format!("{}/", path.trim_end_matches('/'));
                held.remove(&dir);
                if failed.iter().any(|p| p.starts_with(&under)) {
                    crate::remote_checkpoint::forget(&dir);
                } else if let Err(e) = crate::remote_checkpoint::write(&dir, &token) {
                    eprintln!("justrust agent: checkpoint {path}: {e}");
                    crate::remote_checkpoint::forget(&dir);
                } else {
                    held.insert(dir, crate::remote_checkpoint::Held { token, name: path });
                }
            }
            Msg::Verify {
                id,
                path,
                token,
                next,
            } => {
                let dir = at(&path);
                let result =
                    crate::remote_checkpoint::check(&dir, &token, &payload).and_then(|()| {
                        crate::remote_checkpoint::write(&dir, &next).map_err(|e| e.to_string())
                    });
                held.remove(&dir);
                match &result {
                    Ok(()) => {
                        held.insert(
                            dir,
                            crate::remote_checkpoint::Held {
                                token: next,
                                name: path,
                            },
                        );
                    }
                    // Whatever it was, it is not evidence for anyone now.
                    Err(_) => crate::remote_checkpoint::forget(&dir),
                }
                send(
                    &out,
                    &Msg::Verified {
                        id,
                        ok: result.is_ok(),
                        why: result.err().unwrap_or_default(),
                    },
                    &[],
                );
            }
            Msg::Barrier { id } => send(
                &out,
                &Msg::BarrierAck {
                    id,
                    failed: failed.iter().cloned().collect(),
                    disk_full: !disk_full.is_empty(),
                    lost: crate::remote_checkpoint::take_lost(&mut held),
                },
                &[],
            ),
            Msg::Run {
                id,
                cwd,
                args,
                env,
                run_id,
                roots: run_roots,
                ..
            }
            | Msg::RunSync {
                id,
                cwd,
                args,
                env,
                run_id,
                roots: run_roots,
                ..
            } => {
                let received = std::time::Instant::now();
                if sync_run {
                    let lost = crate::remote_checkpoint::take_lost(&mut held);
                    let displaced = crate::remote_checkpoint::blocks_run(&lost, &run_roots);
                    send(
                        &out,
                        &Msg::BarrierAck {
                            id,
                            failed: failed.iter().cloned().collect(),
                            disk_full: !disk_full.is_empty(),
                            lost,
                        },
                        &[],
                    );
                    if displaced && failed.is_empty() {
                        send(
                            &out,
                            &Msg::Error {
                                id,
                                msg: "another connection changed the source mirror, build not started"
                                    .into(),
                            },
                            &[],
                        );
                        continue;
                    }
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
                let admission_gc = admission_gc.clone();
                let publishing = publishing.clone();
                let last_periodic_collection = last_periodic_collection.clone();
                std::thread::spawn(move || {
                    if let Some(free) = low_disk_after_eviction(&roots, &admission_gc) {
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
                    let root = roots.lock().ok().and_then(|r| r.root_of(&cwd));
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
                    let prepared = received.elapsed();
                    let result =
                        start_run(&executable, id, &cwd, &args, &env, &run_id, &out, &pids);
                    let executed = received.elapsed();
                    let publication = publishing.read().unwrap_or_else(|p| p.into_inner());
                    // Keep the build lock and durable uncertainty marker live through
                    // RunFiles and Exit. A concurrent collection may hold the registry
                    // for seconds; releasing protection is not part of Cargo's result.
                    let finish_activity = || {
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
                    };
                    // Both completion paths release activity before periodic collection.
                    let collect = claim_periodic_collection(
                        &last_periodic_collection,
                        std::time::Instant::now(),
                        collection_interval(&env),
                    );
                    let finalized = received.elapsed();
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
                            finish_activity();
                            drop(publication);
                            if collect {
                                evict_now(&roots);
                            }
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
                    let timing = crate::remote_proto::RunTiming {
                        prepare_ms: prepared.as_secs_f64() * 1000.0,
                        execute_ms: (executed - prepared).as_secs_f64() * 1000.0,
                        finalize_ms: (finalized - executed).as_secs_f64() * 1000.0,
                        records_ms: (received.elapsed() - finalized).as_secs_f64() * 1000.0,
                    };
                    send(
                        &out,
                        &Msg::Exit {
                            id,
                            code,
                            timing: Some(timing),
                        },
                        &summary,
                    );
                    finish_activity();
                    drop(publication);
                    if collect {
                        evict_now(&roots);
                    }
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
    drop(publishing.write().unwrap_or_else(|p| p.into_inner()));
    res
}

/// Workspaces cap `build.jobs` for their contributors' laptops (Jcode sets
/// `jobs = 4` for ~15 GB machines), which left 28 of the build machine's 32
/// cores idle. This machine runs only justrust builds, so size jobs for it:
/// by cores up to `GOVERNED_JOBS`, with memory admitted per compiler by the
/// governor (`src/memgov.rs`) instead of a static ~2 GiB per job, which
/// gave 14 jobs on a 96-core, 29-GiB slot. Without the governor the static
/// bound stays. Job count is not part of any fingerprint. An explicit
/// `-j`/`--jobs` or a client-sent `CARGO_BUILD_JOBS` wins.
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
    if memory_governed(env) {
        return Some(cpus.clamp(1, GOVERNED_JOBS));
    }
    let mem_gib = crate::procfs::installed_mb().map(|mb| (mb >> 10) as usize);
    Some(jobs_for(cpus, mem_gib))
}

/// Where more jobs stopped paying on the 96-core slot: cold `check` of a
/// 613-unit workspace took 26.0 s at 14 jobs, 21.1 at 24, 20.7 at 32, 19.4
/// at 48 and 19.5 at 64 (runs 20261126-164410278-472317 to
/// 20261126-164548863-485986). The dependency graph, not the core count,
/// is the limit: about 10 cores were busy on average at every setting.
const GOVERNED_JOBS: usize = 48;

/// The memory governor runs for this machine's builds unless the client or
/// this agent's own environment turns it off (`JUSTRUST_MEMGOV=0`).
fn memory_governed(env: &[(String, String)]) -> bool {
    let set = env
        .iter()
        .find(|(k, _)| k == crate::memgov::ENV)
        .map(|(_, v)| v.clone())
        .or_else(|| std::env::var(crate::memgov::ENV).ok());
    cfg!(target_os = "linux") && set.is_none_or(|v| crate::memgov::on(&v))
}

/// The toolchain a build on this machine uses when the project pins none:
/// the one the control plane names ([`crate::toolchain_policy`], so it can
/// be switched or rolled back there without a client release), else the
/// compiled-in [`crate::depcache_prebuilt::HOSTED_TOOLCHAIN`] when it has
/// never answered, or the stock nightly the prebuilt store started with
/// ([`crate::depcache_prebuilt::STORE_TOOLCHAIN`]) when the jr compiler
/// cannot serve this build ([`hosted_default`]). The prebuild fleet compiles
/// the store for both, and keys include rustc `-vV`, so a machine building
/// with its system stable rustc could never hit the store (0 prebuilt hits
/// in every hosted run before, FINDINGS 55). `None` (leave the choice alone)
/// when the client sent `JUSTRUST_TOOLCHAIN`, the project pins one
/// (`justrust.toml`, or a `rust-toolchain` file rustup would honor), or
/// `JUSTRUST_REMOTE_TOOLCHAIN` says otherwise (`system` keeps the machine's
/// rustc, any other value is used as the default spec).
///
/// `Err` refuses the build: the control plane's toolchain speaks a protocol
/// this justrust does not, and would be the one used.
fn store_toolchain(
    cwd: &Path,
    args: &[String],
    env: &[(String, String)],
    policy: Option<&crate::toolchain_policy::Policy>,
) -> Result<Option<String>, String> {
    let machine = std::env::var("JUSTRUST_REMOTE_TOOLCHAIN").ok();
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) && machine.is_none() {
        return Ok(None);
    }
    if env
        .iter()
        .any(|(k, _)| matches!(k.as_str(), "JUSTRUST_TOOLCHAIN" | "RUSTUP_TOOLCHAIN"))
    {
        return Ok(None);
    }
    // The project's own pin wins over anything the service prefers.
    if crate::toolchain::project_pinned(cwd) {
        return Ok(None);
    }
    let want = match machine {
        Some(m) => m,
        None => {
            let active = policy
                .map(|p| p.toolchain.as_str())
                .unwrap_or(crate::depcache_prebuilt::HOSTED_TOOLCHAIN);
            let want = hosted_default(args, env, active, jr_obtainable(active, env));
            if let Some(p) = policy.filter(|p| p.toolchain == want) {
                p.check()?;
            }
            want.to_owned()
        }
    };
    if matches!(want.trim(), "" | "system" | "off" | "0" | "none") {
        return Ok(None);
    }
    Ok(Some(want))
}

/// The active compiler, unless this build needs something it does not have.
/// Its bundle ships the host standard library only and upstream target
/// libraries must never be mixed into it (`toolchain::add_target`), so a
/// build for another target keeps the stock nightly, which installs official
/// target libraries on demand. The same holds when the jr prefix is neither
/// installed nor downloadable here: a failed install would drop the build
/// to the system rustc and lose every prebuilt hit.
fn hosted_default<'a>(
    args: &[String],
    env: &[(String, String)],
    active: &'a str,
    jr_obtainable: bool,
) -> &'a str {
    let os: Vec<std::ffi::OsString> = args.iter().map(Into::into).collect();
    let env_target = env
        .iter()
        .find(|(k, _)| k == "CARGO_BUILD_TARGET")
        .map(|(_, v)| v.into());
    let cross = crate::record::requested_targets(&os, env_target)
        .iter()
        .any(|t| t != crate::toolchain::TARGET);
    // An official toolchain installs target libraries on demand and needs
    // no toolchain store.
    let official = !active.starts_with("jr-");
    if official || !(cross || !jr_obtainable) {
        active
    } else {
        crate::depcache_prebuilt::STORE_TOOLCHAIN
    }
}

/// Installed already, or a toolchain store is configured to download it
/// from (by this machine or forwarded by the client) and automatic installs
/// are not turned off.
fn jr_obtainable(id: &str, env: &[(String, String)]) -> bool {
    let available = crate::toolchain::Spec::parse(id)
        .ok()
        .and_then(|spec| crate::toolchain::available_for_build(&spec))
        .is_some();
    jr_obtainable_with_local(available, env)
}

fn jr_obtainable_with_local(available: bool, env: &[(String, String)]) -> bool {
    if available {
        return true;
    }
    let var = |name: &str| {
        env.iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
            .or_else(|| std::env::var(name).ok())
    };
    let off = |v: &str| matches!(v.trim(), "" | "0" | "off" | "false" | "no");
    var("JUSTRUST_TOOLCHAIN_DIST").is_some_and(|v| !off(&v))
        && !var("JUSTRUST_TOOLCHAIN_AUTO_INSTALL").is_some_and(|v| off(&v))
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

#[cfg(unix)]
fn valid_canonical_repo(repo: &str) -> bool {
    !repo.is_empty()
        && repo.len() <= 120
        && !repo.contains("..")
        && repo
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Return the shared per-repo incremental cache directory
/// (`~/.justrust/ws/<repo>.incr`) for `cwd` without moving or renaming `cwd`.
/// Keeping `cwd` in place preserves `remote_evict` root inodes and avoids any
/// race with concurrent `rsync` or parallel swarm builds, while `rustc`'s own
/// session-dir locking safely shares the incremental cache across worktrees.
#[cfg(unix)]
fn shared_repo_incremental_dir(cwd: &Path, env: &[(String, String)]) -> Option<PathBuf> {
    if std::env::var("JUSTRUST_CANONICAL_WS").as_deref() == Ok("0") {
        return None;
    }
    let (_, repo) = env.iter().find(|(k, _)| k == "JUSTRUST_CANONICAL_REPO")?;
    if !valid_canonical_repo(repo) || !cwd.is_dir() {
        return None;
    }
    let ws_dir = crate::paths::home().ok()?.join("ws");
    let shared_incr = ws_dir.join(format!("{repo}.incr"));
    std::fs::create_dir_all(&shared_incr).ok()?;
    Some(shared_incr)
}

#[cfg(target_os = "linux")]
fn bwrap_available() -> bool {
    static OK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OK.get_or_init(|| {
        if !Path::new("/usr/bin/bwrap").is_file() {
            return false;
        }
        let uid = unsafe { libc::geteuid() };
        let gid = unsafe { libc::getegid() };
        Command::new("/usr/bin/bwrap")
            .args(["--unshare-user", "--uid"])
            .arg(uid.to_string())
            .arg("--gid")
            .arg(gid.to_string())
            .args(["--dev-bind", "/", "/", "--", "/usr/bin/true"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Return a per-repo canonical mountpoint (`~/.justrust/ws/<repo>`) to
/// bind-mount `cwd` onto inside a private `bwrap` mount namespace.
///
/// Unlike renaming `cwd` in the host namespace, a private bind mount:
/// - never moves or replaces `cwd` (preserving `remote_evict` root inodes),
/// - requires no lock (concurrent `swarm` builds of the same repo run in
///   parallel in separate mount namespaces), and
/// - gives every worktree and checkout of the same repo identical `getcwd()`,
///   `CARGO_MANIFEST_DIR`, and `OUT_DIR` paths so `localcache` and `rustc`
///   incremental CGUs reuse across worktrees even for crates that embed
///   `env!("CARGO_MANIFEST_DIR")`.
#[cfg(target_os = "linux")]
fn bwrap_canonical_mount(cwd: &Path, env: &[(String, String)]) -> Option<PathBuf> {
    // Opt-in via JUSTRUST_CANONICAL_WS=1 until hosted empty-dir mount issue is resolved.
    let enabled = std::env::var("JUSTRUST_CANONICAL_WS").as_deref() == Ok("1")
        || env
            .iter()
            .any(|(k, v)| k == "JUSTRUST_CANONICAL_WS" && v == "1");
    if !enabled {
        return None;
    }
    let (_, repo) = env.iter().find(|(k, _)| k == "JUSTRUST_CANONICAL_REPO")?;
    if !valid_canonical_repo(repo) || !cwd.is_dir() || !cwd.join("Cargo.toml").is_file() {
        return None;
    }
    let canonical = crate::paths::home().ok()?.join("ws").join(repo);
    if cwd == canonical {
        return None;
    }
    if !bwrap_available() {
        return None;
    }
    std::fs::create_dir_all(&canonical).ok()?;
    // Verify the bind mount actually exposes Cargo.toml inside the bwrap
    // namespace before committing to it; fail open to `cwd` otherwise.
    let uid = unsafe { libc::geteuid() };
    let gid = unsafe { libc::getegid() };
    let verified = Command::new("/usr/bin/bwrap")
        .args(["--unshare-user", "--uid"])
        .arg(uid.to_string())
        .arg("--gid")
        .arg(gid.to_string())
        .args(["--dev-bind", "/", "/", "--bind"])
        .arg(cwd)
        .arg(&canonical)
        .arg("--chdir")
        .arg(&canonical)
        .args(["--", "/usr/bin/test", "-f", "Cargo.toml"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !verified {
        eprintln!(
            "justrust agent: bwrap canonical mount probe failed (cwd={}, canonical={}); falling back to cwd",
            cwd.display(),
            canonical.display()
        );
        return None;
    }
    Some(canonical)
}

/// The loopback proxy's URL and the store it serves.
fn prebuilt_proxy() -> Option<&'static (String, String)> {
    static URL: std::sync::OnceLock<Option<(String, String)>> = std::sync::OnceLock::new();
    URL.get_or_init(|| {
        let upstream = crate::depcache_prebuilt::base()?;
        if !upstream.starts_with("https://") && !upstream.starts_with("http://") {
            return None;
        }
        let cache_dir = crate::paths::cache_root()?.join("prebuilt-mirror");
        let url = crate::prebuilt_proxy::start_with(
            crate::depcache_prebuilt::proxy_sources(&upstream),
            cache_dir,
        )?;
        Some((url, upstream))
    })
    .as_ref()
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
    #[cfg(target_os = "linux")]
    let canonical_mount = bwrap_canonical_mount(cwd, env);
    #[cfg(not(target_os = "linux"))]
    let canonical_mount: Option<PathBuf> = None;

    let mut c = if let Some(ref canonical) = canonical_mount {
        #[cfg(target_os = "linux")]
        {
            let uid = unsafe { libc::geteuid() };
            let gid = unsafe { libc::getegid() };
            let mut b = Command::new("/usr/bin/bwrap");
            b.args(["--unshare-user", "--uid"])
                .arg(uid.to_string())
                .arg("--gid")
                .arg(gid.to_string())
                .args(["--dev-bind", "/", "/", "--bind"])
                .arg(cwd)
                .arg(canonical)
                .arg("--chdir")
                .arg(canonical)
                .arg("--")
                .arg(executable)
                .args(args);
            b
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = canonical;
            let mut b = Command::new(executable);
            b.args(args).current_dir(cwd);
            b
        }
    } else {
        let mut b = Command::new(executable);
        b.args(args).current_dir(cwd);
        b
    };
    #[cfg(unix)]
    if let Some(shared_incr) = shared_repo_incremental_dir(cwd, env) {
        c.env("JUSTRUST_SHARED_INCREMENTAL_DIR", shared_incr);
    }
    c.envs(env.iter().map(|(k, v)| (k, v)).filter(|(k, _)| {
        *k != CLIENT_CARGO_HOME
            && *k != crate::remote_proto::CLIENT_UMASK
            && *k != crate::remote_proto::CLIENT_LIVE_CPU
            && *k != "JUSTRUST_CANONICAL_REPO"
    }))
    .env_remove(crate::remote_proto::CLIENT_UMASK)
    .env_remove(crate::remote_proto::CLIENT_LIVE_CPU)
    .env_remove("JUSTRUST_CANONICAL_REPO");
    if canonical_mount.is_some() {
        // An out-of-namespace jr-cargo daemon cannot see this process's private
        // bwrap bind mount; keep Cargo inside the bwrap mount namespace.
        c.env("JUSTRUST_CARGO_DAEMON", "0");
    }
    c.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .detached();
    own_run_lifetime(&mut c);
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
    // Hosted default: answer from rust-analyzer early (docs/early-exit-hosted.md).
    for (k, v) in hosted_early_env(env, std::env::var("EARLY_EXIT_DEFAULT").ok().as_deref()) {
        c.env(k, v);
    }
    let policy = crate::toolchain_policy::current();
    match store_toolchain(cwd, args, env, policy.as_ref()) {
        Ok(Some(tc)) => {
            // A compiler this machine has not built with before: its copy
            // of the store's index predates the fleet's units for it.
            crate::prebuilt_proxy::note_toolchain(tc.as_ref());
            c.env("JUSTRUST_TOOLCHAIN", tc);
        }
        Ok(None) => {}
        Err(refused) => bail!("build not started: {refused}"),
    }
    if let Some(j) = machine_jobs(args, env) {
        c.env("CARGO_BUILD_JOBS", j.to_string());
    }
    // Always on here, also under an explicit job count: that is when a
    // build can ask for more memory than the machine has.
    if memory_governed(env) {
        c.env(crate::memgov::ENV, "1");
    }
    // Fetch every store hit here. The per-fetch overhead the cost model
    // assumes (700 ms, a laptop under agent load) declined 487 of 529
    // available hits on a 32-job build machine, where fetches overlap:
    // cold `check -p jcode-tui` 37.5-41.3 s costed vs 35.7-38.7 s fetching
    // all (FINDINGS 65). A client-sent value wins.
    if !env.iter().any(|(k, _)| k == "JUSTRUST_PREBUILT_ALWAYS") {
        c.env("JUSTRUST_PREBUILT_ALWAYS", "1");
    }
    // Serve the public prebuilt store through the agent's local read-only
    // loopback proxy (pooled upstream TLS + local NVMe cache) when the caller
    // has not overridden JUSTRUST_PREBUILT.
    if !env.iter().any(|(k, _)| k == "JUSTRUST_PREBUILT")
        && let Some((proxy_url, store)) = prebuilt_proxy()
    {
        c.env("JUSTRUST_PREBUILT", proxy_url);
        // The proxy's port differs per agent. Shims name their copy of the
        // store's index after the store, so sessions share one.
        c.env(crate::depcache_prebuilt::STORE_ENV, store);
    }
    // Enable workspace-portable localcache on hosted builds so unchanged
    // workspace crates reuse across git worktrees and fresh checkouts of the
    // same account without recompiling from scratch.
    if !env.iter().any(|(k, _)| k == "JUSTRUST_LOCALCACHE_PORTABLE") {
        c.env("JUSTRUST_LOCALCACHE_PORTABLE", "1");
    }
    // cargo and rustup live in ~/.cargo/bin, and Homebrew tools (zstd, git)
    // live in /opt/homebrew/bin or /usr/local/bin on macOS, which a non-login
    // ssh shell may not have on PATH.
    if let Some(home) = dirs::home_dir() {
        let path = std::env::var("PATH").unwrap_or_default();
        c.env(
            "PATH",
            format!(
                "{0}/.cargo/bin:/opt/homebrew/bin:{0}/.homebrew/bin:/usr/local/bin:{path}",
                home.display()
            ),
        );
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
    let watchdog_done: Arc<(Mutex<bool>, std::sync::Condvar)> = Arc::default();
    let watchdog = {
        let done = watchdog_done.clone();
        let out = out.clone();
        let pgid = child.id() as i32;
        let limit = max_run_duration();
        std::thread::spawn(move || {
            let guard = done.0.lock().unwrap();
            let (guard, timeout) = done
                .1
                .wait_timeout_while(guard, limit, |finished| !*finished)
                .unwrap();
            if !timeout.timed_out() || *guard {
                return;
            }
            drop(guard);
            send(
                &out,
                &Msg::Out { id, stream: 2 },
                format!(
                    "\njustrust: hosted run exceeded the {} min wall-clock limit and was stopped\n",
                    limit.as_secs() / 60
                )
                .as_bytes(),
            );
            // SAFETY: signalling our own child's process group.
            unsafe {
                libc::kill(-pgid, libc::SIGINT);
            }
            let guard = done.0.lock().unwrap();
            let (guard, _) = done
                .1
                .wait_timeout_while(guard, RUN_KILL_GRACE, |finished| !*finished)
                .unwrap();
            if !*guard {
                // SAFETY: as above; the group ignored SIGINT.
                unsafe {
                    libc::kill(-pgid, libc::SIGKILL);
                }
            }
        })
    };
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
    let st = child.wait();
    *watchdog_done.0.lock().unwrap() = true;
    watchdog_done.1.notify_all();
    let _ = watchdog.join();
    let st = st?;
    pids.lock().unwrap().remove(&id);
    untrack_pgid(child.id() as i32);
    use std::os::unix::process::ExitStatusExt;
    Ok(st.code().unwrap_or_else(|| 128 + st.signal().unwrap_or(0)))
}

/// Default wall-clock cap for one hosted run. A hung test or runaway build
/// otherwise holds a shared machine (and its billing) indefinitely: one run
/// went for an hour on 2026-11-12. The host operator overrides it with
/// `JUSTRUST_REMOTE_MAX_RUN_SECS`. The client cannot raise it.
const DEFAULT_MAX_RUN: std::time::Duration = std::time::Duration::from_secs(30 * 60);
const RUN_KILL_GRACE: std::time::Duration = std::time::Duration::from_secs(15);

fn max_run_duration() -> std::time::Duration {
    parse_max_run(
        std::env::var("JUSTRUST_REMOTE_MAX_RUN_SECS")
            .ok()
            .as_deref(),
    )
}

fn parse_max_run(v: Option<&str>) -> std::time::Duration {
    v.and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(std::time::Duration::from_secs)
        .unwrap_or(DEFAULT_MAX_RUN)
}

/// Early exit defaults for a hosted run: `JUSTRUST_EARLY=auto` and
/// `JUSTRUST_EARLY_EXIT=1` unless the client chose either itself or the
/// machine's kill switch (`EARLY_EXIT_DEFAULT=0`, from the VM bootstrap) is off.
fn hosted_early_env(
    env: &[(String, String)],
    kill_switch: Option<&str>,
) -> Vec<(&'static str, &'static str)> {
    if matches!(kill_switch, Some("0" | "off" | "false" | "no")) {
        return Vec::new();
    }
    let has = |k: &str| env.iter().any(|(key, _)| key == k);
    let mut out = Vec::new();
    if has("JUSTRUST_EARLY_EXIT") {
        return out;
    }
    out.push(("JUSTRUST_EARLY_EXIT", "1"));
    if !has("JUSTRUST_EARLY") {
        out.push(("JUSTRUST_EARLY", "auto"));
    }
    // Match the VM's 30 minute idle stop instead of the 3 h laptop default.
    if !has("JUSTRUST_EARLY_IDLE_SECS") {
        out.push(("JUSTRUST_EARLY_IDLE_SECS", "1800"));
    }
    out
}

/// A detached recorder is still owned by this agent. Normal disconnects
/// signal the run group, but SIGKILL (or an early fatal exit before PID
/// publication) cannot execute that cleanup.
fn own_run_lifetime(command: &mut Command) {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let owner = std::process::id() as libc::pid_t;
        // SAFETY: only async-signal-safe operations run between fork and exec.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // The parent may have died before prctl armed the signal.
                if libc::getppid() != owner {
                    libc::_exit(128 + libc::SIGTERM);
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = command;
}

#[cfg(test)]
mod tests {
    #[test]
    fn hosted_early_defaults_respect_client_and_kill_switch() {
        use super::hosted_early_env;
        let none: Vec<(String, String)> = Vec::new();
        let d = hosted_early_env(&none, None);
        assert!(d.contains(&("JUSTRUST_EARLY_EXIT", "1")));
        assert!(d.contains(&("JUSTRUST_EARLY", "auto")));
        assert!(hosted_early_env(&none, Some("0")).is_empty());
        assert_eq!(hosted_early_env(&none, Some("1")), d);
        let opt_out = vec![("JUSTRUST_EARLY_EXIT".to_string(), "0".to_string())];
        assert!(hosted_early_env(&opt_out, None).is_empty());
        let mode = vec![("JUSTRUST_EARLY".to_string(), "1".to_string())];
        assert!(
            !hosted_early_env(&mode, None)
                .iter()
                .any(|(k, _)| *k == "JUSTRUST_EARLY")
        );
    }

    #[test]
    fn max_run_defaults_and_overrides() {
        assert_eq!(super::parse_max_run(None), super::DEFAULT_MAX_RUN);
        assert_eq!(super::parse_max_run(Some("0")), super::DEFAULT_MAX_RUN);
        assert_eq!(super::parse_max_run(Some("x")), super::DEFAULT_MAX_RUN);
        assert_eq!(
            super::parse_max_run(Some(" 90 ")),
            std::time::Duration::from_secs(90)
        );
    }

    #[test]
    fn periodic_collection_is_bounded_and_zero_restores_every_run() {
        let last = std::sync::Mutex::new(None);
        let now = std::time::Instant::now();
        let interval = std::time::Duration::from_secs(60);
        assert!(super::claim_periodic_collection(&last, now, interval));
        assert!(!super::claim_periodic_collection(&last, now, interval));
        assert!(!super::claim_periodic_collection(
            &last,
            now + std::time::Duration::from_secs(59),
            interval,
        ));
        assert!(super::claim_periodic_collection(
            &last,
            now + interval,
            interval
        ));
        assert!(super::claim_periodic_collection(
            &last,
            now + interval,
            std::time::Duration::ZERO,
        ));
    }

    #[test]
    fn periodic_collection_claim_is_single_flight_for_simultaneous_runs() {
        let last = std::sync::Mutex::new(None);
        let now = std::time::Instant::now();
        let claimed = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        super::claim_periodic_collection(
                            &last,
                            now,
                            std::time::Duration::from_secs(60),
                        )
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| usize::from(worker.join().unwrap()))
                .sum::<usize>()
        });
        assert_eq!(claimed, 1);
        assert!(!super::claim_periodic_collection(
            &last,
            now,
            std::time::Duration::from_secs(60),
        ));
    }

    #[test]
    fn periodic_collection_interval_is_bounded_and_request_scoped() {
        assert_eq!(super::collection_interval(&[]).as_secs(), 60);
        for (input, expected) in [
            ("0", 0),
            ("1", 1),
            ("3600", 3600),
            ("3601", 60),
            ("-1", 60),
            ("bad", 60),
        ] {
            let env = vec![("JUSTRUST_REMOTE_EVICT_INTERVAL_SECS".into(), input.into())];
            assert_eq!(super::collection_interval(&env).as_secs(), expected);
        }
    }
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn detached_run_dies_with_its_agent() {
        const FLAG: &str = "JUSTRUST_TEST_OWNER_FIXTURE";
        if let Some(dir) = std::env::var_os(FLAG) {
            let dir = PathBuf::from(dir);
            let mut command = Command::new("/bin/sh");
            command.args(["-c", "trap 'echo stopped > stopped; exit 0' TERM; echo ready > ready; i=0; while [ $i -lt 250 ]; do /bin/sleep 0.02; i=$((i+1)); done"])
                .current_dir(&dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).detached();
            own_run_lifetime(&mut command);
            let mut child = command.spawn().unwrap();
            for _ in 0..250 {
                if dir.join("ready").exists() {
                    // Deliberately skip normal __agent cleanup.
                    std::process::exit(0);
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            // The successful branch intentionally exits to exercise parent death.
            // On timeout we still own the child and must reap it before panicking.
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned child did not become ready");
        }
        let dir = std::env::temp_dir().join(format!("jr-owner-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let nested = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "remote_agent::tests::detached_run_dies_with_its_agent",
                "--nocapture",
            ])
            .env(FLAG, &dir)
            .output()
            .unwrap();
        assert!(
            nested.status.success(),
            "{}{}",
            String::from_utf8_lossy(&nested.stdout),
            String::from_utf8_lossy(&nested.stderr)
        );
        for _ in 0..250 {
            if dir.join("stopped").exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let stopped = dir.join("stopped").exists();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(
            stopped,
            "detached recorder survived its owner without receiving SIGTERM"
        );
    }

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
    fn unpinned_projects_build_with_the_hosted_toolchain() {
        let d = std::env::temp_dir().join(format!("jr-store-tc-{}", std::process::id()));
        let sub = d.join("a/b");
        std::fs::create_dir_all(&sub).unwrap();
        // The dist store makes the jr compiler obtainable on any machine.
        let dist = (
            "JUSTRUST_TOOLCHAIN_DIST".to_string(),
            "https://store.example".to_string(),
        );
        let linux = cfg!(all(target_os = "linux", target_arch = "x86_64"));
        let want = |id: &str| Ok(linux.then(|| id.to_owned()));
        // The control plane has never answered: the compiled-in default.
        assert_eq!(
            store_toolchain(&sub, &[], std::slice::from_ref(&dist), None),
            want(crate::depcache_prebuilt::HOSTED_TOOLCHAIN)
        );
        // It names one: that is what builds use, with no client release.
        let policy = |id: &str, protocol| crate::toolchain_policy::Policy {
            toolchain: id.into(),
            protocol,
        };
        let ok = *crate::toolchain_policy::SUPPORTED.end();
        let next = policy("jr-0123456789ab", ok);
        assert_eq!(
            store_toolchain(&sub, &[], std::slice::from_ref(&dist), Some(&next)),
            want("jr-0123456789ab")
        );
        let official = policy("nightly-2026-11-01", ok);
        assert_eq!(
            store_toolchain(&sub, &[], &[], Some(&official)),
            want("nightly-2026-11-01")
        );
        // One this justrust cannot drive: refuse, never guess.
        let too_new = policy("jr-0123456789ab", ok + 1);
        let refused = store_toolchain(&sub, &[], std::slice::from_ref(&dist), Some(&too_new));
        if linux {
            let msg = refused.unwrap_err();
            assert!(msg.contains("`justrust update`"), "{msg}");
        }
        // ...unless the build would not use it anyway.
        let cross = [
            "build".to_string(),
            "--target=wasm32-unknown-unknown".to_string(),
        ];
        assert_eq!(
            store_toolchain(&sub, &cross, std::slice::from_ref(&dist), Some(&too_new)),
            want(crate::depcache_prebuilt::STORE_TOOLCHAIN)
        );
        // The client chose one: keep it.
        let env = [("JUSTRUST_TOOLCHAIN".to_string(), "system".to_string())];
        assert_eq!(store_toolchain(&sub, &[], &env, Some(&too_new)), Ok(None));
        let env = [("RUSTUP_TOOLCHAIN".to_string(), "1.90.0".to_string())];
        assert_eq!(store_toolchain(&sub, &[], &env, Some(&next)), Ok(None));
        // The project pins one (rustup file or justrust.toml): keep it,
        // whatever the control plane says.
        std::fs::write(d.join("a/rust-toolchain.toml"), "").unwrap();
        assert_eq!(
            store_toolchain(&sub, &[], std::slice::from_ref(&dist), Some(&next)),
            Ok(None)
        );
        assert_eq!(
            store_toolchain(&sub, &[], &[dist], Some(&too_new)),
            Ok(None)
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn hosted_default_is_the_active_toolchain_unless_it_cannot_serve_the_build() {
        use crate::depcache_prebuilt::{HOSTED_TOOLCHAIN, STORE_TOOLCHAIN};
        let args = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(HOSTED_TOOLCHAIN.starts_with("jr-"));
        let jr = "jr-0123456789ab";
        assert_eq!(hosted_default(&args(&["check"]), &[], jr, true), jr);
        // The host triple spelled out is still a host build.
        let host = args(&["build", "--target", crate::toolchain::TARGET]);
        assert_eq!(hosted_default(&host, &[], jr, true), jr);
        // The jr bundle has no other target's standard library.
        for cross in [
            args(&["build", "--target", "wasm32-unknown-unknown"]),
            args(&["zigbuild", "--target=aarch64-apple-ios"]),
        ] {
            assert_eq!(hosted_default(&cross, &[], jr, true), STORE_TOOLCHAIN);
            // An official toolchain installs target libraries itself.
            assert_eq!(
                hosted_default(&cross, &[], "nightly-2026-11-01", false),
                "nightly-2026-11-01"
            );
        }
        let env = [(
            "CARGO_BUILD_TARGET".to_string(),
            "thumbv7em-none-eabi".to_string(),
        )];
        assert_eq!(
            hosted_default(&args(&["check"]), &env, jr, true),
            STORE_TOOLCHAIN
        );
        // A target after `--` belongs to the test binary, not cargo.
        let passthrough = args(&["test", "--", "--target", "x"]);
        assert_eq!(hosted_default(&passthrough, &[], jr, true), jr);
        // Not installed and nowhere to download it from.
        assert_eq!(
            hosted_default(&args(&["check"]), &[], jr, false),
            STORE_TOOLCHAIN
        );
    }

    #[test]
    fn already_available_build_toolchains_need_no_download_authority() {
        let env = vec![
            ("JUSTRUST_TOOLCHAIN_DIST".into(), "off".into()),
            ("JUSTRUST_TOOLCHAIN_AUTO_INSTALL".into(), "0".into()),
        ];
        // Both private installs and host-authorized public bundles enter this
        // same branch. Disabling mutation/downloads must not force an upstream
        // fallback when the exact compiler is already present for builds.
        assert!(jr_obtainable_with_local(true, &env));
        assert!(!jr_obtainable_with_local(false, &env));
        assert_eq!(
            hosted_default(
                &["check".to_string()],
                &env,
                "jr-ac20de5e7921",
                jr_obtainable_with_local(true, &env),
            ),
            "jr-ac20de5e7921"
        );
    }

    #[test]
    fn jr_needs_a_store_and_automatic_installs_when_absent() {
        // Never installed anywhere.
        let id = "jr-00000000000000aa";
        if std::env::var_os("JUSTRUST_TOOLCHAIN_DIST").is_some() {
            return; // this machine can always get it
        }
        let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
        assert!(!jr_obtainable(id, &[]));
        assert!(jr_obtainable(
            id,
            &[kv("JUSTRUST_TOOLCHAIN_DIST", "https://s.example")]
        ));
        assert!(!jr_obtainable(id, &[kv("JUSTRUST_TOOLCHAIN_DIST", "off")]));
        assert!(!jr_obtainable(
            id,
            &[
                kv("JUSTRUST_TOOLCHAIN_DIST", "https://s.example"),
                kv("JUSTRUST_TOOLCHAIN_AUTO_INSTALL", "0"),
            ]
        ));
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

    #[test]
    fn governed_jobs_follow_cores_not_memory() {
        let kv = |v: &str| [(crate::memgov::ENV.to_string(), v.to_string())];
        let args = ["check".to_string()];
        let cpus = std::thread::available_parallelism().unwrap().get();
        if cfg!(target_os = "linux") {
            assert!(memory_governed(&kv("1")));
            assert_eq!(machine_jobs(&args, &kv("1")), Some(cpus.min(GOVERNED_JOBS)));
        }
        // Turned off, the static memory bound is back.
        assert!(!memory_governed(&kv("0")));
        let mem_gib = crate::procfs::installed_mb().map(|mb| (mb >> 10) as usize);
        assert_eq!(machine_jobs(&args, &kv("0")), Some(jobs_for(cpus, mem_gib)));
    }
}
