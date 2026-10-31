//! Per-agent build slots: private cargo target directories so concurrent
//! agents never wait on each other's build-directory lock, and one agent's
//! upstream edit does not churn everyone else's incremental state.
//!
//! ```text
//! <target-dir>/justrust-slots/
//!   0/            a full cargo target dir (passed as --target-dir)
//!   0.lock        flock held for the whole run by the agent using slot 0
//!   0.json        owner session, last use, seed info
//!   1/ ...
//! ```
//!
//! Rules:
//! - Only agent-mode `check`, `test`, and `clippy`. `build` and `run` stay on
//!   the shared target dir because people and scripts expect the binaries in
//!   `target/<profile>/` (set `JUSTRUST_SLOTS_ALL=1` to include them).
//! - Never when the user picked a target dir (`CARGO_TARGET_DIR`,
//!   `CARGO_BUILD_TARGET_DIR`, `--target-dir`, `--config ...target-dir...`).
//! - A session (`JCODE_SESSION_ID`, else the Unix session id) sticks to its
//!   slot so its incremental cache stays warm. If that slot is busy it takes
//!   a free one, creating up to `JUSTRUST_SLOTS` (default 4) slots. If all are
//!   busy it falls back to the shared dir and never waits.
//! - A new slot is seeded with a reflink copy of the shared profile dir, so
//!   dependencies are fresh and the copy shares disk extents. Without reflink
//!   support (not btrfs/xfs) slots are disabled: a full copy would be huge.
//! - Fails open: any error means "use the shared target dir".

use crate::paths;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::fs::File;
use std::os::fd::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

const DEFAULT_SLOTS: usize = 4;
/// Subcommands that use slots by default.
const SLOT_SUBCOMMANDS: &[&str] = &["check", "c", "test", "t", "clippy"];
/// Added with `JUSTRUST_SLOTS_ALL=1`.
const SLOT_SUBCOMMANDS_ALL: &[&str] = &["build", "b", "run", "r"];
/// Incremental caches untouched for longer than this are not copied when
/// seeding. On the Desktop, 129 of 1970 crate dirs (11 of 181 GB) were used
/// in the last two days. A crate without a copied cache is still fresh; it
/// only loses incremental reuse the next time it is edited.
const INCREMENTAL_MAX_AGE: Duration = Duration::from_secs(3 * 24 * 3600);

/// What happened with slots for one run. Stored in the run summary.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct SlotInfo {
    /// Slot number, or None when the run used the shared target dir.
    pub slot: Option<usize>,
    /// The target dir cargo used.
    pub target_dir: String,
    /// "sticky" (this session's slot), "new", "reused" (taken from another
    /// session), or "fallback" (all slots busy, used the shared dir).
    pub how: String,
    /// Seconds spent seeding the slot before cargo started.
    #[serde(default)]
    pub seed_secs: f64,
    /// Slots that were busy when this run looked for one.
    #[serde(default)]
    pub busy: usize,
    #[serde(default)]
    pub capacity: usize,
    /// Seconds spent bringing units the shared dir built since the seed
    /// into the slot (see `refresh`).
    #[serde(default)]
    pub refresh_secs: f64,
    /// Units copied by that refresh.
    #[serde(default)]
    pub refreshed_units: usize,
    /// Seconds waited for an identical run in another session to finish
    /// (then this run reused its slot instead of compiling the same crates).
    #[serde(default)]
    pub join_wait_secs: f64,
}

impl SlotInfo {
    pub fn report_line(&self) -> String {
        match self.slot {
            Some(n) => {
                let mut s = format!("slot {n} ({}) {}", self.how, self.target_dir);
                if self.seed_secs > 0.0 {
                    s.push_str(&format!(", seeded in {:.1}s", self.seed_secs));
                }
                if self.refreshed_units > 0 {
                    s.push_str(&format!(
                        ", {} newer units from shared in {:.1}s",
                        self.refreshed_units, self.refresh_secs
                    ));
                }
                if self.how == "joined" {
                    s.push_str(&format!(
                        ", waited {:.1}s for the identical build running there",
                        self.join_wait_secs
                    ));
                }
                s
            }
            None => format!(
                "shared {} (all {} slots busy)",
                self.target_dir, self.capacity
            ),
        }
    }
}

/// A held slot. The flock is released when this is dropped (or the process exits).
pub struct Slot {
    pub info: SlotInfo,
    /// Cargo arguments with `--target-dir <slot>` added.
    pub args: Vec<OsString>,
    _lock: Option<File>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct SlotMeta {
    owner: String,
    last_used: f64,
    #[serde(default)]
    seeded: bool,
    #[serde(default)]
    seed_secs: f64,
    #[serde(default)]
    seeded_from: String,
    /// `shared_stamp` of the shared profile dir at the last seed or refresh.
    #[serde(default)]
    shared_stamp: f64,
}

fn subcommand_index(args: &[OsString]) -> Option<usize> {
    let sub = crate::record::subcommand(args)?;
    args.iter().position(|a| a.to_string_lossy() == sub)
}

fn env_set(k: &str) -> bool {
    std::env::var_os(k).is_some_and(|v| !v.is_empty())
}

/// Whether this run may use a slot, from the arguments and environment only.
pub fn eligible(args: &[OsString]) -> bool {
    if std::env::var("JUSTRUST_SLOTS").as_deref() == Ok("0") {
        return false;
    }
    if env_set("CARGO_TARGET_DIR") || env_set("CARGO_BUILD_TARGET_DIR") {
        return false;
    }
    let all = std::env::var_os("JUSTRUST_SLOTS_ALL").is_some_and(|v| v != "0");
    eligible_args(args, all)
}

fn eligible_args(args: &[OsString], all: bool) -> bool {
    let Some(sub) = crate::record::subcommand(args) else {
        return false;
    };
    if !SLOT_SUBCOMMANDS.contains(&sub.as_str())
        && !(all && SLOT_SUBCOMMANDS_ALL.contains(&sub.as_str()))
    {
        return false;
    }
    let cargo_args = args
        .iter()
        .map(|a| a.to_string_lossy())
        .take_while(|a| a != "--");
    for a in cargo_args {
        if a == "--target-dir" || a.starts_with("--target-dir=") || a.contains("target-dir") {
            return false;
        }
    }
    true
}

/// The profile directory name cargo will use (`debug`, `release`, custom),
/// prefixed by `<triple>/` when a single `--target` is given.
pub fn profile_dir(args: &[OsString]) -> PathBuf {
    let args: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .take_while(|a| a != "--")
        .collect();
    let mut profile = "debug".to_owned();
    let mut target = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let value = |name: &str| -> Option<String> {
            if a == name {
                args.get(i + 1).cloned()
            } else {
                a.strip_prefix(&format!("{name}=")).map(str::to_owned)
            }
        };
        if a == "--release" || a == "-r" {
            profile = "release".to_owned();
        } else if let Some(p) = value("--profile") {
            profile = match p.as_str() {
                "dev" | "test" => "debug".to_owned(),
                "bench" => "release".to_owned(),
                other => other.to_owned(),
            };
        } else if let Some(t) = value("--target") {
            target = Some(t);
        }
        i += 1;
    }
    match target {
        Some(t) => Path::new(&t).join(profile),
        None => PathBuf::from(profile),
    }
}

/// The session a run belongs to. Runs from one agent session share a slot.
pub fn session_key() -> String {
    if let Ok(s) = std::env::var("JCODE_SESSION_ID")
        && !s.is_empty()
    {
        return s;
    }
    // SAFETY: getsid has no memory-safety preconditions.
    let sid = unsafe { libc::getsid(0) };
    format!("sid-{sid}")
}

fn capacity() -> usize {
    std::env::var("JUSTRUST_SLOTS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_SLOTS)
}

/// The shared target dir for the workspace, as cargo resolves it (config,
/// workspace root). About 20 ms.
fn shared_target_dir(args: &[OsString]) -> Option<PathBuf> {
    let cargo = paths::real_cargo().ok()?;
    let mut cmd = Command::new(cargo);
    cmd.args([
        "metadata",
        "--no-deps",
        "--format-version",
        "1",
        "--offline",
    ]);
    let mut it = args.iter().map(|a| a.to_string_lossy().into_owned());
    while let Some(a) = it.next() {
        if a == "--" {
            break;
        }
        if a == "--manifest-path" {
            cmd.arg(&a);
            if let Some(v) = it.next() {
                cmd.arg(v);
            }
        } else if a.starts_with("--manifest-path=") {
            cmd.arg(&a);
        }
    }
    let out = cmd.stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    v.get("target_directory")?.as_str().map(PathBuf::from)
}

fn try_lock(path: &Path) -> Option<File> {
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    // SAFETY: flock on a valid fd.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

fn read_meta(path: &Path) -> Option<SlotMeta> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// How the run picked a slot from the pool.
#[derive(Debug, PartialEq)]
enum Pick {
    Sticky(usize),
    New(usize),
    Reused(usize),
    /// Another run of the identical command held this slot; we waited for it.
    Joined(usize),
}

/// Pick a slot: this session's own, else an unused number below the cap,
/// else the least recently used existing one. `metas[i]` is None for slot
/// numbers that do not exist yet. `free(i)` tries to lock slot i.
fn pick(
    metas: &[Option<SlotMeta>],
    me: &str,
    mut free: impl FnMut(usize) -> bool,
) -> (Option<Pick>, usize) {
    let mut busy = 0;
    let mut tried = vec![false; metas.len()];
    if let Some(i) = metas
        .iter()
        .position(|m| m.as_ref().is_some_and(|m| m.owner == me))
    {
        tried[i] = true;
        if free(i) {
            return (Some(Pick::Sticky(i)), busy);
        }
        busy += 1;
    }
    for i in 0..metas.len() {
        if metas[i].is_none() && !tried[i] {
            tried[i] = true;
            if free(i) {
                return (Some(Pick::New(i)), busy);
            }
            busy += 1;
        }
    }
    let mut order: Vec<usize> = (0..metas.len()).filter(|&i| !tried[i]).collect();
    order.sort_by(|&a, &b| {
        let t = |i: usize| metas[i].as_ref().map_or(0.0, |m| m.last_used);
        t(a).total_cmp(&t(b))
    });
    for i in order {
        if free(i) {
            return (Some(Pick::Reused(i)), busy);
        }
        busy += 1;
    }
    (None, busy)
}

fn reflink_supported(dir: &Path) -> bool {
    let a = dir.join(format!(".reflink-probe-{}", std::process::id()));
    let b = dir.join(format!(".reflink-probe-{}.copy", std::process::id()));
    let ok = std::fs::write(&a, b"probe").is_ok()
        && Command::new("cp")
            .arg("--reflink=always")
            .arg(&a)
            .arg(&b)
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    let _ = std::fs::remove_file(&a);
    let _ = std::fs::remove_file(&b);
    ok
}

fn cp_reflink(srcs: &[PathBuf], dst: &Path, extra: &[&str]) -> bool {
    if srcs.is_empty() {
        return true;
    }
    srcs.chunks(500).all(|chunk| {
        Command::new("cp")
            .arg("-a")
            .arg("--reflink=always")
            .args(extra)
            .args(chunk)
            .arg(dst)
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    })
}

/// Reflink-copy many entries with several `cp` processes. Copying is bound
/// by per-file metadata work in the filesystem, which parallelizes: on the
/// Desktop target (28k deps files) 6 processes took 5.0s vs 8.8s for one.
const SEED_JOBS: usize = 6;

fn cp_parallel(jobs: &[(PathBuf, PathBuf)], extra: &[&str]) -> bool {
    use std::collections::BTreeMap;
    let mut by_dst: BTreeMap<&Path, Vec<PathBuf>> = BTreeMap::new();
    for (src, dst) in jobs {
        by_dst.entry(dst.as_path()).or_default().push(src.clone());
    }
    // Small batches pulled from a shared queue by SEED_JOBS workers, so one
    // huge directory does not leave the other workers idle.
    let mut work: Vec<(Vec<PathBuf>, PathBuf)> = Vec::new();
    for (dst, srcs) in by_dst {
        for chunk in srcs.chunks(256) {
            work.push((chunk.to_vec(), dst.to_path_buf()));
        }
    }
    let next = std::sync::atomic::AtomicUsize::new(0);
    let ok = std::sync::atomic::AtomicBool::new(true);
    std::thread::scope(|s| {
        for _ in 0..SEED_JOBS.min(work.len()) {
            s.spawn(|| {
                use std::sync::atomic::Ordering::Relaxed;
                while let Some((srcs, dst)) = work.get(next.fetch_add(1, Relaxed)) {
                    if !ok.load(Relaxed) {
                        return;
                    }
                    if !cp_reflink(srcs, dst, extra) {
                        ok.store(false, Relaxed);
                    }
                }
            });
        }
    });
    ok.into_inner()
}

fn children(dir: &Path, max_age: Option<Duration>) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let now = SystemTime::now();
    rd.flatten()
        .filter(|e| {
            max_age.is_none_or(|max| {
                e.metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| now.duration_since(t).ok())
                    .is_some_and(|age| age <= max)
            })
        })
        .map(|e| e.path())
        .collect()
}

/// Reflink-copy the shared profile dir into the slot's profile dir.
///
/// Order matters because the shared dir may be in use by another cargo:
/// fingerprints are copied before the artifacts they describe, so a copied
/// fingerprint never claims an artifact newer than the copied one. An artifact
/// newer than its fingerprint only makes cargo rebuild that unit.
fn seed(shared: &Path, slot: &Path) -> bool {
    let stage = |subs: &[(&str, Option<Duration>)]| -> bool {
        let mut jobs = Vec::new();
        for (sub, max_age) in subs {
            let dst = slot.join(sub);
            if std::fs::create_dir_all(&dst).is_err() {
                return false;
            }
            for src in children(&shared.join(sub), *max_age) {
                jobs.push((src, dst.clone()));
            }
        }
        cp_parallel(&jobs, &[])
    };
    stage(&[(".fingerprint", None)])
        && stage(&[
            ("build", None),
            ("deps", None),
            ("examples", None),
            ("incremental", Some(INCREMENTAL_MAX_AGE)),
        ])
}

/// The unit hash in an artifact file name: `libfoo-<16 hex>.rlib`,
/// `foo-<16 hex>.d`, `foo-<16 hex>` (test binary). Crate names in `deps/`
/// use underscores, so the first `-<16 hex>` followed by `.` or the end is it.
fn artifact_hash(name: &str) -> Option<&str> {
    let b = name.as_bytes();
    for (i, _) in name.match_indices('-') {
        let h = name.get(i + 1..i + 17)?;
        let end_ok = b.get(i + 17).is_none_or(|&c| c == b'.');
        if end_ok
            && h.bytes()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
        {
            return Some(h);
        }
    }
    None
}

/// The unit hash of a `.fingerprint/<name>-<hash>` or `build/<name>-<hash>` dir.
fn dir_hash(name: &str) -> Option<&str> {
    let (_, h) = name.rsplit_once('-')?;
    (h.len() == 16 && h.bytes().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

/// Unit dirs under `.fingerprint`, keyed by name, with the newest mtime of
/// the files inside (cargo rewrites them in place, so the dir's own mtime
/// does not move).
fn fingerprint_units(profile: &Path) -> std::collections::HashMap<String, SystemTime> {
    let mut out = std::collections::HashMap::new();
    let Ok(rd) = std::fs::read_dir(profile.join(".fingerprint")) else {
        return out;
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if dir_hash(&name).is_none() {
            continue;
        }
        let newest = std::fs::read_dir(e.path())
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|f| f.metadata().ok()?.modified().ok())
            .max()
            .unwrap_or(SystemTime::UNIX_EPOCH);
        out.insert(name, newest);
    }
    out
}

/// Units to bring from the shared dir into a slot: those the slot lacks and
/// those the shared dir rebuilt after the slot's copy. A unit the slot built
/// itself more recently is never touched.
fn stale_units(
    shared: &std::collections::HashMap<String, SystemTime>,
    slot: &std::collections::HashMap<String, SystemTime>,
) -> (Vec<String>, Vec<String>) {
    let mut missing = Vec::new();
    let mut newer = Vec::new();
    for (name, &t) in shared {
        match slot.get(name) {
            None => missing.push(name.clone()),
            Some(&mine) if t > mine => newer.push(name.clone()),
            Some(_) => {}
        }
    }
    missing.sort();
    newer.sort();
    (missing, newer)
}

/// Mtime of the shared dirs cargo writes into whenever it builds anything.
/// If unchanged since the last refresh there is nothing new to copy.
fn shared_stamp(shared: &Path) -> f64 {
    [".fingerprint", "deps", "build"]
        .iter()
        .filter_map(|d| std::fs::metadata(shared.join(d)).ok()?.modified().ok())
        .filter_map(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs_f64())
        .fold(0.0, f64::max)
}

/// Whether two copies of one unit's fingerprint dir record the same state:
/// the hash files (no extension, not `dep-*` or `invoked.timestamp`) match.
fn same_fingerprint(a: &Path, b: &Path) -> bool {
    let hashes = |d: &Path| -> Option<Vec<(String, Vec<u8>)>> {
        let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(d)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                (!n.contains('.') && !n.starts_with("dep-"))
                    .then(|| Some((n, std::fs::read(e.path()).ok()?)))
                    .flatten()
            })
            .collect();
        v.sort();
        Some(v)
    };
    matches!((hashes(a), hashes(b)), (Some(x), Some(y)) if !x.is_empty() && x == y)
}

/// What a refresh did.
#[derive(Debug, Default, PartialEq)]
struct Refresh {
    missing: usize,
    newer: usize,
    files: usize,
}

/// Bring a slot up to date with units the shared dir built since the slot
/// was seeded (Cargo.lock bump, gpui rev, an upstream crate rebuilt by
/// `cargo build`). The unit is the safe granule: its fingerprint dir, its
/// `build/` dir, and every `deps/` file carrying its hash, copied together
/// with mtimes preserved. Cargo's fingerprints then decide freshness as
/// usual: a copied unit whose inputs or dependencies differ in the slot is
/// simply rebuilt.
///
/// The caller holds the slot's flock and a shared flock on the shared dir's
/// build lock, so neither side is being written. Crash safety: a replaced
/// unit's old fingerprint is deleted first and the new fingerprint is copied
/// last, so an interrupted refresh leaves units without a fingerprint, which
/// cargo always rebuilds.
fn refresh(shared: &Path, slot: &Path) -> Option<Refresh> {
    let (a, b) = std::thread::scope(|s| {
        let a = s.spawn(|| fingerprint_units(shared));
        let b = fingerprint_units(slot);
        (a.join().unwrap_or_default(), b)
    });
    let (missing, mut newer) = stale_units(&a, &b);
    // Same unit, rebuilt in both places to the same state: copying would only
    // bump mtimes and make the slot's dependents look stale.
    newer.retain(|u| {
        !same_fingerprint(
            &shared.join(".fingerprint").join(u),
            &slot.join(".fingerprint").join(u),
        )
    });
    let mut r = Refresh {
        missing: missing.len(),
        newer: newer.len(),
        files: 0,
    };
    if missing.is_empty() && newer.is_empty() {
        return Some(r);
    }
    let units: Vec<&String> = missing.iter().chain(&newer).collect();
    let hashes: std::collections::HashSet<&str> =
        units.iter().filter_map(|u| dir_hash(u)).collect();

    // Drop the slot's older copies of units being replaced.
    if !newer.is_empty() {
        let replaced: std::collections::HashSet<&str> =
            newer.iter().filter_map(|u| dir_hash(u)).collect();
        for u in &newer {
            std::fs::remove_dir_all(slot.join(".fingerprint").join(u)).ok();
            std::fs::remove_dir_all(slot.join("build").join(u)).ok();
        }
        for f in std::fs::read_dir(slot.join("deps"))
            .into_iter()
            .flatten()
            .flatten()
        {
            if artifact_hash(&f.file_name().to_string_lossy()).is_some_and(|h| replaced.contains(h))
            {
                let p = f.path();
                if p.is_dir() {
                    std::fs::remove_dir_all(&p).ok();
                } else {
                    std::fs::remove_file(&p).ok();
                }
            }
        }
    }

    // Artifacts first.
    let mut jobs = Vec::new();
    for sub in ["build", "deps"] {
        let dst = slot.join(sub);
        std::fs::create_dir_all(&dst).ok()?;
        for e in std::fs::read_dir(shared.join(sub))
            .into_iter()
            .flatten()
            .flatten()
        {
            let name = e.file_name();
            let name = name.to_string_lossy();
            let h = if sub == "build" {
                dir_hash(&name)
            } else {
                artifact_hash(&name)
            };
            if h.is_some_and(|h| hashes.contains(h)) {
                jobs.push((e.path(), dst.clone()));
            }
        }
    }
    r.files = jobs.len();
    let fp_dst = slot.join(".fingerprint");
    let fp_jobs: Vec<(PathBuf, PathBuf)> = units
        .iter()
        .map(|u| (shared.join(".fingerprint").join(u), fp_dst.clone()))
        .collect();
    // --remove-destination: never write through a hardlink (deps files are
    // hardlinked to uplifted outputs) or into a mapped binary.
    if !cp_parallel(&jobs, &["--remove-destination"]) {
        return None;
    }
    // Then the fingerprints that vouch for them.
    if !cp_parallel(&fp_jobs, &["--remove-destination"]) {
        for u in &units {
            std::fs::remove_dir_all(fp_dst.join(u)).ok();
        }
        return None;
    }
    Some(r)
}

fn try_lock_shared(path: &Path) -> Option<File> {
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    // SAFETY: flock on a valid fd.
    let r = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_SH | libc::LOCK_NB) };
    (r == 0).then_some(f)
}

/// Identity of a build request for joining: working dir, cargo arguments
/// (before `--`: test filters do not change what is compiled), and the
/// environment that changes compilation. Equal keys compile the same units
/// from the same sources unless a file changes in between, and then cargo's
/// own fingerprints still catch it, because the joiner runs cargo itself.
fn request_key(args: &[OsString]) -> String {
    let mut k = std::env::current_dir()
        .map(|d| d.to_string_lossy().into_owned())
        .unwrap_or_default();
    for a in args
        .iter()
        .map(|a| a.to_string_lossy())
        .take_while(|a| a != "--")
    {
        k.push('\0');
        k.push_str(&a);
    }
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(n, _)| {
            n.starts_with("CARGO_") && !n.starts_with("CARGO_TERM_") && n != "CARGO_HOME"
                || [
                    "RUSTFLAGS",
                    "RUSTDOCFLAGS",
                    "RUSTC_WRAPPER",
                    "RUSTC",
                    "RUSTUP_TOOLCHAIN",
                ]
                .contains(&n.as_str())
        })
        .collect();
    env.sort();
    for (n, v) in env {
        k.push_str(&format!("\0{n}={v}"));
    }
    k
}

/// Longest wait for an identical run before compiling in parallel anyway.
fn join_max_wait() -> Duration {
    Duration::from_secs_f64(env_f64("JUSTRUST_JOIN_WAIT", 300.0).max(0.0))
}

/// If a slot is held right now by a run with the same request key, wait for
/// it to finish and take that slot. Two identical builds in parallel compile
/// the same crates twice and compete for the CPU (pair 15s vs 10-12s when
/// the second waited on the shared dir's lock, FINDINGS section 7). Waiting
/// and then running cargo in the finished slot costs one no-op cargo.
fn join_identical(root: &Path, cap: usize, key: &str) -> (Option<(usize, File)>, f64) {
    if std::env::var("JUSTRUST_JOIN").as_deref() == Ok("0") {
        return (None, 0.0);
    }
    let Some(n) = (0..cap).find(|n| {
        std::fs::read_to_string(root.join(format!("{n}.req"))).is_ok_and(|k| k == key)
            && try_lock_probe(&root.join(format!("{n}.lock")))
    }) else {
        return (None, 0.0);
    };
    eprintln!(
        "justrust: an identical build is running in slot {n}; waiting for it instead of \
         compiling the same crates twice (JUSTRUST_JOIN=0 disables)"
    );
    let t = Instant::now();
    let max = join_max_wait();
    loop {
        if let Some(f) = try_lock(&root.join(format!("{n}.lock"))) {
            return (Some((n, f)), t.elapsed().as_secs_f64());
        }
        if t.elapsed() >= max {
            eprintln!(
                "justrust: still running after {:.0}s, building in parallel",
                max.as_secs_f64()
            );
            return (None, t.elapsed().as_secs_f64());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// True when someone holds the lock (non-blocking probe, released at once).
fn try_lock_probe(path: &Path) -> bool {
    match try_lock(path) {
        Some(_) => false,
        None => path.exists(),
    }
}

/// Choose and lock a slot for this run. None means "run exactly as before".
pub fn acquire(args: &[OsString]) -> Option<Slot> {
    if !eligible(args) {
        return None;
    }
    let target = shared_target_dir(args)?;
    let profile = profile_dir(args);
    let shared_profile = target.join(&profile);
    // Nothing to seed from: building deps into a slot from zero would cost
    // as much as the shared build and then double the disk use.
    if !shared_profile.join(".fingerprint").is_dir() {
        return None;
    }
    let root = target.join("justrust-slots");
    std::fs::create_dir_all(&root).ok()?;
    if !root.join(".reflink-ok").exists() {
        if !reflink_supported(&root) {
            return None;
        }
        let _ = std::fs::write(root.join(".reflink-ok"), b"");
    }

    let cap = capacity();
    let me = session_key();
    let key = request_key(args);
    let (joined, join_wait) = join_identical(&root, cap, &key);
    let metas: Vec<Option<SlotMeta>> = (0..cap)
        .map(|i| read_meta(&root.join(format!("{i}.json"))))
        .collect();
    let mut locks: Vec<Option<File>> = (0..cap).map(|_| None).collect();
    let (picked, busy) = match joined {
        Some((n, lock)) => {
            locks[n] = Some(lock);
            (Some(Pick::Joined(n)), 0)
        }
        None => pick(&metas, &me, |i| {
            locks[i] = try_lock(&root.join(format!("{i}.lock")));
            locks[i].is_some()
        }),
    };

    let Some(picked) = picked else {
        return Some(Slot {
            info: SlotInfo {
                slot: None,
                target_dir: target.to_string_lossy().into_owned(),
                how: "fallback".to_owned(),
                seed_secs: 0.0,
                busy,
                capacity: cap,
                ..Default::default()
            },
            args: args.to_vec(),
            _lock: None,
        });
    };
    let (n, how) = match picked {
        Pick::Sticky(n) => (n, "sticky"),
        Pick::New(n) => (n, "new"),
        Pick::Reused(n) => (n, "reused"),
        Pick::Joined(n) => (n, "joined"),
    };
    let lock = locks[n].take();
    drop(locks);
    let slot_dir = root.join(n.to_string());
    let meta_path = root.join(format!("{n}.json"));
    let mut meta = metas[n]
        .as_ref()
        .map_or_else(SlotMeta::default, |m| SlotMeta {
            owner: m.owner.clone(),
            last_used: m.last_used,
            seeded: m.seeded,
            seed_secs: m.seed_secs,
            seeded_from: m.seeded_from.clone(),
            shared_stamp: m.shared_stamp,
        });

    let mut seed_secs = 0.0;
    let mut refresh_secs = 0.0;
    let mut refreshed_units = 0;
    let slot_profile = slot_dir.join(&profile);
    // Shared lock on the shared dir's build lock: cargo takes it exclusively
    // while building, so holding it means the shared dir is not being written.
    // Never wait for it: a busy shared dir means skip, the slot is still valid.
    let shared_guard = try_lock_shared(&shared_profile.join(".cargo-build-lock"));
    let stamp = shared_stamp(&shared_profile);
    if !meta.seeded || !slot_profile.join(".fingerprint").is_dir() {
        eprintln!(
            "justrust: seeding build slot {n} from {} (reflink copy, once per slot)",
            shared_profile.display()
        );
        let t = Instant::now();
        let ok = seed(&shared_profile, &slot_profile);
        seed_secs = t.elapsed().as_secs_f64();
        if !ok {
            eprintln!("justrust: seeding slot {n} failed, using the shared target dir");
            return None;
        }
        meta.seeded = true;
        meta.seed_secs = seed_secs;
        meta.seeded_from = shared_profile.to_string_lossy().into_owned();
        if shared_guard.is_some() {
            meta.shared_stamp = stamp;
        }
    } else if shared_guard.is_some()
        && stamp > meta.shared_stamp
        && std::env::var("JUSTRUST_SLOT_REFRESH").as_deref() != Ok("0")
    {
        let t = Instant::now();
        match refresh(&shared_profile, &slot_profile) {
            Some(r) => {
                refresh_secs = t.elapsed().as_secs_f64();
                refreshed_units = r.missing + r.newer;
                meta.shared_stamp = stamp;
                if refreshed_units > 0 {
                    eprintln!(
                        "justrust: slot {n}: copied {} new and {} rebuilt units ({} files) \
                         from the shared target dir in {refresh_secs:.1}s",
                        r.missing, r.newer, r.files
                    );
                }
            }
            None => eprintln!("justrust: slot {n}: refresh from the shared dir failed, skipped"),
        }
    }
    drop(shared_guard);
    // A joined slot stays its owner's sticky slot.
    if how != "joined" {
        meta.owner = me;
    }
    meta.last_used = paths::now();
    if let Ok(bytes) = serde_json::to_vec_pretty(&meta) {
        let _ = std::fs::write(&meta_path, bytes);
    }
    // What this slot is building now, so an identical request can join it.
    let _ = std::fs::write(root.join(format!("{n}.req")), &key);

    let mut new_args = args.to_vec();
    let at = subcommand_index(args).map_or(new_args.len(), |i| i + 1);
    new_args.insert(at, OsString::from("--target-dir"));
    new_args.insert(at + 1, slot_dir.clone().into_os_string());
    Some(Slot {
        info: SlotInfo {
            slot: Some(n),
            target_dir: slot_dir.to_string_lossy().into_owned(),
            how: how.to_owned(),
            seed_secs,
            busy,
            capacity: cap,
            refresh_secs,
            refreshed_units,
            join_wait_secs: join_wait,
        },
        args: new_args,
        _lock: lock,
    })
}

/// Default GC limits. Slots idle longer than this are removed.
const GC_MAX_AGE_DAYS: f64 = 7.0;
/// Default budget for the bytes slots do not share with the shared dir
/// (btrfs exclusive). On the Desktop four slots used 0.3 to 8 GB each.
const GC_MAX_GB: f64 = 40.0;
/// Opportunistic GC runs at most this often per workspace.
const GC_INTERVAL: Duration = Duration::from_secs(3600);

/// Disk use of one slot: exclusive bytes (not shared with any other file)
/// and total, from `btrfs filesystem du`, else apparent size from `du` for
/// both (no sharing information off btrfs).
fn slot_size(dir: &Path) -> Option<(u64, u64)> {
    let out = Command::new("btrfs")
        .args(["filesystem", "du", "-s", "--raw"])
        .arg(dir)
        .stderr(Stdio::null())
        .output();
    if let Ok(out) = out
        && out.status.success()
        && let Some(line) = String::from_utf8_lossy(&out.stdout).lines().last()
    {
        let f: Vec<u64> = line
            .split_whitespace()
            .take(2)
            .filter_map(|x| x.parse().ok())
            .collect();
        if let [total, excl] = f[..] {
            return Some((excl, total));
        }
    }
    let out = Command::new("du")
        .arg("-sb")
        .arg(dir)
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let b: u64 = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .next()?
        .parse()
        .ok()?;
    Some((b, b))
}

/// One slot as seen by GC.
#[derive(Debug, Clone, PartialEq)]
struct GcSlot {
    n: usize,
    idle_secs: f64,
    exclusive: u64,
    in_use: bool,
}

/// Which slots to remove: free slots idle past `max_age`, then free slots
/// least recently used first until exclusive bytes fit `budget`. A slot in
/// use is never chosen (it still counts toward the budget).
fn gc_plan(slots: &[GcSlot], max_age_secs: f64, budget: u64) -> Vec<usize> {
    let mut remove: Vec<usize> = slots
        .iter()
        .filter(|s| !s.in_use && s.idle_secs > max_age_secs)
        .map(|s| s.n)
        .collect();
    let mut used: u64 = slots
        .iter()
        .filter(|s| !remove.contains(&s.n))
        .map(|s| s.exclusive)
        .sum();
    // A slot used in the last hour is some agent's warm cache: over budget
    // or not, leave it.
    let mut lru: Vec<&GcSlot> = slots
        .iter()
        .filter(|s| !s.in_use && !remove.contains(&s.n) && s.idle_secs >= GC_INTERVAL.as_secs_f64())
        .collect();
    lru.sort_by(|a, b| b.idle_secs.total_cmp(&a.idle_secs));
    for s in lru {
        if used <= budget {
            break;
        }
        used -= s.exclusive;
        remove.push(s.n);
    }
    remove.sort_unstable();
    remove
}

fn env_f64(k: &str, default: f64) -> f64 {
    std::env::var(k)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Slot numbers present under `root` (dir or meta file).
fn slot_numbers(root: &Path) -> Vec<usize> {
    let mut v: Vec<usize> = std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".json").unwrap_or(&name).parse().ok()
        })
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// Remove a free slot. The slot's flock is held while its dir is renamed
/// away and its meta deleted, so no agent can pick it half-removed; the
/// slow delete of the renamed tree happens after.
fn remove_slot(root: &Path, n: usize) -> bool {
    let Some(lock) = try_lock(&root.join(format!("{n}.lock"))) else {
        return false;
    };
    let trash = root.join(format!(".trash-{n}-{}", std::process::id()));
    let dir = root.join(n.to_string());
    let moved = !dir.exists() || std::fs::rename(&dir, &trash).is_ok();
    if moved {
        std::fs::remove_file(root.join(format!("{n}.json"))).ok();
        std::fs::remove_file(root.join(format!("{n}.req"))).ok();
    }
    drop(lock);
    if trash.exists() {
        std::fs::remove_dir_all(&trash).ok();
    }
    moved
}

/// Garbage-collect the slots under `root`. Returns the removed slot numbers
/// and the exclusive bytes they held.
pub fn gc(root: &Path) -> (Vec<usize>, u64) {
    // Leftovers of an interrupted removal.
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        if e.file_name().to_string_lossy().starts_with(".trash-") {
            std::fs::remove_dir_all(e.path()).ok();
        }
    }
    let now = paths::now();
    let ns = slot_numbers(root);
    let sizes = slot_sizes(root, &ns);
    let slots: Vec<GcSlot> = ns
        .iter()
        .zip(sizes)
        .map(|(&n, size)| {
            let in_use = try_lock(&root.join(format!("{n}.lock"))).is_none();
            let meta = read_meta(&root.join(format!("{n}.json"))).unwrap_or_default();
            let exclusive = size.map_or(0, |s| s.0);
            GcSlot {
                n,
                idle_secs: (now - meta.last_used).max(0.0),
                exclusive,
                in_use,
            }
        })
        .collect();
    let max_age = env_f64("JUSTRUST_SLOTS_MAX_AGE_DAYS", GC_MAX_AGE_DAYS) * 86400.0;
    let budget = (env_f64("JUSTRUST_SLOTS_MAX_GB", GC_MAX_GB) * 1e9) as u64;
    let mut freed = 0;
    let mut removed = Vec::new();
    for n in gc_plan(&slots, max_age, budget) {
        if remove_slot(root, n) {
            freed += slots.iter().find(|s| s.n == n).map_or(0, |s| s.exclusive);
            removed.push(n);
        }
    }
    (removed, freed)
}

/// After a run: start a detached GC of this workspace's slots if none ran
/// in the last hour. Never blocks or fails the build.
pub fn maybe_gc_background(info: &SlotInfo) {
    if std::env::var("JUSTRUST_SLOTS_GC").as_deref() == Ok("0") {
        return;
    }
    let target = Path::new(&info.target_dir);
    let root = match info.slot {
        Some(_) => match target.parent() {
            Some(p) => p.to_path_buf(),
            None => return,
        },
        None => target.join("justrust-slots"),
    };
    if !gc_due(&root, SystemTime::now()) {
        return;
    }
    // Claim the hour before spawning so concurrent runs start only one GC.
    if std::fs::write(root.join(".gc-stamp"), b"").is_err() {
        return;
    }
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    use std::os::unix::process::CommandExt;
    let _ = Command::new(me)
        .arg("slots")
        .arg("--gc-root")
        .arg(&root)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn();
}

fn gc_due(root: &Path, now: SystemTime) -> bool {
    if !root.is_dir() {
        return false;
    }
    match std::fs::metadata(root.join(".gc-stamp")).and_then(|m| m.modified()) {
        Ok(t) => now.duration_since(t).is_ok_and(|d| d >= GC_INTERVAL),
        Err(_) => true,
    }
}

fn gb(b: u64) -> String {
    format!("{:.1} GB", b as f64 / 1e9)
}

/// `slot_size` of several slots at once (each is a few seconds of btrfs
/// metadata walking on a 170 GB slot; btrfs serializes them, so this mainly
/// helps the `du` fallback). Results are cached in `.sizes.json` for listing.
fn slot_sizes(root: &Path, ns: &[usize]) -> Vec<Option<(u64, u64)>> {
    let sizes: Vec<Option<(u64, u64)>> = std::thread::scope(|s| {
        let hs: Vec<_> = ns
            .iter()
            .map(|n| s.spawn(move || slot_size(&root.join(n.to_string()))))
            .collect();
        hs.into_iter().map(|h| h.join().ok().flatten()).collect()
    });
    let cache: SizeMap = ns
        .iter()
        .zip(&sizes)
        .filter_map(|(n, s)| Some((n.to_string(), (*s)?)))
        .collect();
    if let Ok(b) = serde_json::to_vec(&(paths::now(), cache)) {
        let _ = std::fs::write(root.join(".sizes.json"), b);
    }
    sizes
}

/// Per-slot `(exclusive, total)` bytes, keyed by slot number.
type SizeMap = std::collections::BTreeMap<String, (u64, u64)>;

/// Sizes from the last GC or listing, and when they were measured.
fn cached_sizes(root: &Path) -> Option<(f64, SizeMap)> {
    serde_json::from_slice(&std::fs::read(root.join(".sizes.json")).ok()?).ok()
}

/// `justrust slots`: list the slots of the current workspace, or remove them.
pub fn command(
    clean: bool,
    run_gc: bool,
    du: bool,
    gc_root: Option<PathBuf>,
) -> anyhow::Result<()> {
    if let Some(root) = gc_root {
        gc(&root);
        return Ok(());
    }
    let target = shared_target_dir(&[])
        .ok_or_else(|| anyhow::anyhow!("not in a cargo workspace (cargo metadata failed)"))?;
    let root = target.join("justrust-slots");
    if !root.is_dir() {
        println!("no build slots under {}", target.display());
        return Ok(());
    }
    if run_gc {
        let (removed, freed) = gc(&root);
        println!(
            "gc: removed {} slot(s) {:?}, freed about {} exclusive",
            removed.len(),
            removed,
            gb(freed)
        );
    }
    let mut n = 0;
    let mut total_excl = 0;
    let ns = slot_numbers(&root);
    // Measuring is seconds per slot on btrfs: use the sizes the last GC
    // recorded unless asked or there are none.
    let cached = cached_sizes(&root).filter(|_| !du && !run_gc);
    let sizes_age = cached.as_ref().map(|c| (paths::now() - c.0).max(0.0));
    let sizes = if clean {
        vec![None; ns.len()]
    } else if let Some((_, c)) = &cached {
        ns.iter().map(|n| c.get(&n.to_string()).copied()).collect()
    } else {
        slot_sizes(&root, &ns)
    };
    for (&i, size) in ns.iter().zip(sizes) {
        let dir = root.join(i.to_string());
        let meta_path = root.join(format!("{i}.json"));
        n += 1;
        let lock = try_lock(&root.join(format!("{i}.lock")));
        let in_use = lock.is_none();
        let meta = read_meta(&meta_path).unwrap_or_default();
        if clean {
            drop(lock);
            if remove_slot(&root, i) {
                println!("slot {i}: removed");
            } else {
                println!("slot {i}: in use, kept");
            }
            continue;
        }
        let age = (paths::now() - meta.last_used).max(0.0);
        total_excl += size.map_or(0, |s| s.0);
        println!(
            "slot {i}: {}  owner {}  last used {} ago  {}  seeded in {:.1}s  {}",
            if in_use { "IN USE" } else { "free  " },
            meta.owner,
            human_age(age),
            size.map_or("size ?".to_owned(), |(e, t)| format!(
                "{} exclusive of {}",
                gb(e),
                gb(t)
            )),
            meta.seed_secs,
            dir.display()
        );
    }
    if n == 0 {
        println!("no build slots under {}", target.display());
    } else if !clean {
        println!(
            "{} exclusive in total{}; the rest shares disk extents with {} (reflink copies).\n\
             GC (hourly, after runs): free slots idle over {} days, then least recently used \
             until exclusive use is under {} GB (JUSTRUST_SLOTS_MAX_AGE_DAYS, \
             JUSTRUST_SLOTS_MAX_GB). `--gc` runs it now, `--du` re-measures sizes, \
             `--clean` removes all free slots.",
            gb(total_excl),
            sizes_age.map_or(String::new(), |a| format!(
                " (sizes measured {} ago)",
                human_age(a)
            )),
            target.display(),
            env_f64("JUSTRUST_SLOTS_MAX_AGE_DAYS", GC_MAX_AGE_DAYS),
            env_f64("JUSTRUST_SLOTS_MAX_GB", GC_MAX_GB),
        );
    }
    Ok(())
}

fn human_age(secs: f64) -> String {
    if secs < 120.0 {
        format!("{secs:.0}s")
    } else if secs < 7200.0 {
        format!("{:.0}m", secs / 60.0)
    } else if secs < 172800.0 {
        format!("{:.0}h", secs / 3600.0)
    } else {
        format!("{:.0}d", secs / 86400.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    fn meta(owner: &str, last_used: f64) -> Option<SlotMeta> {
        Some(SlotMeta {
            owner: owner.to_owned(),
            last_used,
            seeded: true,
            ..Default::default()
        })
    }

    #[test]
    fn eligibility_respects_user_target_dir_and_subcommand() {
        // Env vars are process-global; only test argument rules here.
        let eligible = |a: &[OsString]| eligible_args(a, false);
        assert!(eligible_args(&os(&["build"]), true));
        assert!(eligible(&os(&["test", "-p", "x", "--", "filter"])));
        assert!(eligible(&os(&["check"])));
        assert!(eligible(&os(&["clippy", "--all-targets"])));
        assert!(!eligible(&os(&["check", "--target-dir", "/tmp/x"])));
        assert!(!eligible(&os(&["check", "--target-dir=/tmp/x"])));
        assert!(!eligible(&os(&[
            "--config",
            "build.target-dir=\"t\"",
            "check"
        ])));
        assert!(!eligible(&os(&["build"])));
        assert!(!eligible(&os(&["metadata"])));
        // Arguments after `--` belong to the test binary.
        assert!(eligible(&os(&["test", "--", "--target-dir"])));
    }

    #[test]
    fn profile_dir_from_args() {
        assert_eq!(profile_dir(&os(&["test"])), PathBuf::from("debug"));
        assert_eq!(
            profile_dir(&os(&["build", "--release"])),
            PathBuf::from("release")
        );
        assert_eq!(
            profile_dir(&os(&["check", "--profile", "test"])),
            PathBuf::from("debug")
        );
        assert_eq!(
            profile_dir(&os(&["check", "--profile=fast"])),
            PathBuf::from("fast")
        );
        assert_eq!(
            profile_dir(&os(&["check", "--target", "wasm32-unknown-unknown"])),
            PathBuf::from("wasm32-unknown-unknown/debug")
        );
        assert_eq!(
            profile_dir(&os(&["test", "--", "--release"])),
            PathBuf::from("debug")
        );
    }

    #[test]
    fn pick_prefers_own_slot_then_new_then_lru() {
        let metas = vec![meta("a", 10.0), meta("b", 5.0), None, None];
        assert_eq!(pick(&metas, "b", |_| true), (Some(Pick::Sticky(1)), 0));
        assert_eq!(pick(&metas, "c", |_| true), (Some(Pick::New(2)), 0));
        // Own slot busy: take a new one.
        assert_eq!(pick(&metas, "b", |i| i != 1), (Some(Pick::New(2)), 1));
        // Pool full of other sessions' slots: least recently used free one.
        let full = vec![meta("a", 10.0), meta("b", 5.0), meta("c", 7.0)];
        assert_eq!(pick(&full, "z", |_| true), (Some(Pick::Reused(1)), 0));
        assert_eq!(pick(&full, "z", |i| i == 0), (Some(Pick::Reused(0)), 2));
        // Everything busy: fall back.
        assert_eq!(pick(&full, "a", |_| false), (None, 3));
    }

    #[test]
    fn target_dir_inserted_after_subcommand() {
        let args = os(&["+nightly", "test", "-p", "x", "--", "f"]);
        assert_eq!(subcommand_index(&args), Some(1));
    }

    #[test]
    fn seed_copies_fingerprints_deps_and_recent_incremental() {
        let base = std::env::temp_dir().join(format!("justrust-slot-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let shared = base.join("shared/debug");
        for d in [".fingerprint/a-1", "deps", "build/x", "incremental/a-1/s-1"] {
            std::fs::create_dir_all(shared.join(d)).unwrap();
        }
        std::fs::write(shared.join("deps/liba-1.rlib"), b"rlib").unwrap();
        std::fs::write(shared.join(".fingerprint/a-1/lib-a"), b"fp").unwrap();
        std::fs::write(shared.join("incremental/a-1/s-1/q.bin"), b"q").unwrap();
        if !reflink_supported(&base) {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let slot = base.join("slots/0/debug");
        assert!(seed(&shared, &slot));
        assert_eq!(
            std::fs::read(slot.join("deps/liba-1.rlib")).unwrap(),
            b"rlib"
        );
        assert!(slot.join(".fingerprint/a-1/lib-a").is_file());
        assert!(slot.join("incremental/a-1/s-1/q.bin").is_file());
        // mtimes are preserved, which cargo's freshness check depends on.
        let mt = |p: &Path| std::fs::metadata(p).unwrap().modified().unwrap();
        assert_eq!(
            mt(&shared.join("deps/liba-1.rlib")),
            mt(&slot.join("deps/liba-1.rlib"))
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn gc_plan_reaps_old_then_lru_over_budget() {
        let h = 3600.0;
        let s = |n, idle_h: f64, gb: u64, in_use| GcSlot {
            n,
            idle_secs: idle_h * h,
            exclusive: gb * 1_000_000_000,
            in_use,
        };
        let week = 7.0 * 24.0 * h;
        let budget = 10_000_000_000;
        // Old and free: removed. Old but in use: kept.
        let slots = [
            s(0, 200.0, 1, false),
            s(1, 200.0, 1, true),
            s(2, 1.5, 1, false),
        ];
        assert_eq!(gc_plan(&slots, week, budget), [0]);
        // Over budget: least recently used free slots first, until it fits.
        let slots = [
            s(0, 5.0, 6, false),
            s(1, 30.0, 4, false),
            s(2, 2.0, 5, false),
            s(3, 50.0, 3, true),
        ];
        // 18 GB > 10: drop slot 1 (30h idle) -> 14, then slot 0 (5h) -> 8.
        assert_eq!(gc_plan(&slots, week, budget), [0, 1]);
        // Slots used in the last hour are never reaped for budget.
        let slots = [s(0, 0.2, 50, false), s(1, 0.5, 50, false)];
        assert!(gc_plan(&slots, week, budget).is_empty());
    }

    #[test]
    fn gc_removes_free_slots_and_keeps_locked_ones() {
        let root = std::env::temp_dir().join(format!("justrust-gc-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let old = paths::now() - 30.0 * 86400.0;
        for n in 0..3 {
            std::fs::create_dir_all(root.join(format!("{n}/debug/deps"))).unwrap();
            std::fs::write(root.join(format!("{n}/debug/deps/x")), b"x").unwrap();
            let meta = SlotMeta {
                owner: format!("s{n}"),
                last_used: if n == 2 { paths::now() } else { old },
                seeded: true,
                ..Default::default()
            };
            std::fs::write(
                root.join(format!("{n}.json")),
                serde_json::to_vec(&meta).unwrap(),
            )
            .unwrap();
        }
        let held = try_lock(&root.join("1.lock")).unwrap();
        let (removed, _) = gc(&root);
        assert_eq!(removed, [0]);
        assert!(!root.join("0").exists() && !root.join("0.json").exists());
        assert!(root.join("1/debug/deps/x").exists());
        assert!(root.join("2/debug/deps/x").exists());
        drop(held);
        assert_eq!(slot_numbers(&root), [1, 2]);
        // Hourly gate.
        assert!(gc_due(&root, SystemTime::now()));
        std::fs::write(root.join(".gc-stamp"), b"").unwrap();
        assert!(!gc_due(&root, SystemTime::now()));
        assert!(gc_due(&root, SystemTime::now() + GC_INTERVAL));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn join_waits_for_identical_running_build_only() {
        let root = std::env::temp_dir().join(format!("justrust-join-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let key = request_key(&os(&["test", "-p", "x", "--", "filter"]));
        // Test filters do not change what is compiled.
        assert_eq!(key, request_key(&os(&["test", "-p", "x", "--", "other"])));
        assert_ne!(key, request_key(&os(&["test", "-p", "y"])));
        std::fs::write(root.join("1.req"), &key).unwrap();
        // Slot 1 not running: nothing to join.
        assert!(join_identical(&root, 4, &key).0.is_none());
        let held = try_lock(&root.join("1.lock")).unwrap();
        // Different request: no join even though slot 1 is running.
        assert!(join_identical(&root, 4, "other").0.is_none());
        let t = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            drop(held);
        });
        let (joined, waited) = join_identical(&root, 4, &key);
        assert_eq!(joined.map(|j| j.0), Some(1));
        assert!(waited >= 0.2, "waited {waited}");
        t.join().unwrap();
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unit_hashes_from_names() {
        assert_eq!(
            artifact_hash("libgpui-01e6fc7e6520e628.rlib"),
            Some("01e6fc7e6520e628")
        );
        assert_eq!(
            artifact_hash("jcode_desktop_ui-034d5c77386f7d52"),
            Some("034d5c77386f7d52")
        );
        assert_eq!(
            artifact_hash("libfoo_bar-0123456789abcdef.so"),
            Some("0123456789abcdef")
        );
        assert_eq!(artifact_hash("jcode_desktop_ui.d"), None);
        assert_eq!(artifact_hash("rmetaIfln9S"), None);
        assert_eq!(dir_hash("gpui-01e6fc7e6520e628"), Some("01e6fc7e6520e628"));
        assert_eq!(
            dir_hash("jcode-desktop-ui-01be5818b91178de"),
            Some("01be5818b91178de")
        );
        assert_eq!(dir_hash("serde"), None);
    }

    #[test]
    fn stale_units_never_replace_newer_slot_units() {
        use std::collections::HashMap;
        let t = |s: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(s);
        let shared: HashMap<String, SystemTime> = [
            ("a-0000000000000001", t(10)),
            ("b-0000000000000002", t(20)),
            ("c-0000000000000003", t(30)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let slot: HashMap<String, SystemTime> = [
            ("b-0000000000000002", t(25)), // slot rebuilt it later: keep
            ("c-0000000000000003", t(5)),  // shared rebuilt it later: take
            ("d-0000000000000004", t(1)),  // slot only: keep
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let (missing, newer) = stale_units(&shared, &slot);
        assert_eq!(missing, ["a-0000000000000001"]);
        assert_eq!(newer, ["c-0000000000000003"]);
    }

    #[test]
    fn refresh_copies_whole_units_and_keeps_slot_work() {
        let base =
            std::env::temp_dir().join(format!("justrust-refresh-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        if !reflink_supported(&base) {
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let shared = base.join("shared/debug");
        let slot = base.join("slot/debug");
        let unit = |root: &Path, name: &str, hash: &str, body: &str| {
            let fp = root.join(format!(".fingerprint/{name}-{hash}"));
            std::fs::create_dir_all(&fp).unwrap();
            std::fs::write(fp.join(format!("lib-{name}")), body).unwrap();
            std::fs::create_dir_all(root.join("deps")).unwrap();
            std::fs::write(root.join(format!("deps/lib{name}-{hash}.rlib")), body).unwrap();
            std::fs::write(root.join(format!("deps/{name}-{hash}.d")), body).unwrap();
        };
        unit(&shared, "old", "1111111111111111", "shared");
        unit(&slot, "old", "1111111111111111", "slot");
        unit(&slot, "mine", "3333333333333333", "slot");
        std::thread::sleep(Duration::from_millis(20));
        unit(&shared, "new", "2222222222222222", "shared");
        std::fs::create_dir_all(shared.join("build/new-2222222222222222")).unwrap();
        std::fs::write(shared.join("build/new-2222222222222222/output"), "o").unwrap();
        // Slot's copy of `old` is the newest: must survive.
        std::thread::sleep(Duration::from_millis(20));
        unit(&slot, "old", "1111111111111111", "slot2");

        let r = refresh(&shared, &slot).unwrap();
        assert_eq!((r.missing, r.newer, r.files), (1, 0, 3));
        let read = |p: &str| std::fs::read_to_string(slot.join(p)).unwrap();
        assert_eq!(read("deps/libnew-2222222222222222.rlib"), "shared");
        assert_eq!(read("deps/new-2222222222222222.d"), "shared");
        assert_eq!(read(".fingerprint/new-2222222222222222/lib-new"), "shared");
        assert_eq!(read("build/new-2222222222222222/output"), "o");
        assert_eq!(read("deps/libold-1111111111111111.rlib"), "slot2");
        assert_eq!(read("deps/libmine-3333333333333333.rlib"), "slot");

        // Shared rebuilds `old` later: the whole unit is replaced.
        std::thread::sleep(Duration::from_millis(20));
        unit(&shared, "old", "1111111111111111", "shared2");
        let r = refresh(&shared, &slot).unwrap();
        assert_eq!((r.missing, r.newer), (0, 1));
        assert_eq!(read("deps/libold-1111111111111111.rlib"), "shared2");
        assert_eq!(read(".fingerprint/old-1111111111111111/lib-old"), "shared2");
        // Nothing left to do.
        assert_eq!(refresh(&shared, &slot).unwrap(), Refresh::default());
        // Shared rebuilt `mine` to the identical state: not copied (copying
        // would only bump mtimes and dirty the slot's dependents).
        std::thread::sleep(Duration::from_millis(20));
        unit(&shared, "mine", "3333333333333333", "slot");
        assert_eq!(refresh(&shared, &slot).unwrap(), Refresh::default());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn report_line_mentions_slot_and_seed() {
        let i = SlotInfo {
            slot: Some(2),
            target_dir: "/w/target/justrust-slots/2".into(),
            how: "new".into(),
            seed_secs: 12.5,
            busy: 1,
            capacity: 4,
            ..Default::default()
        };
        assert_eq!(
            i.report_line(),
            "slot 2 (new) /w/target/justrust-slots/2, seeded in 12.5s"
        );
        let i = SlotInfo {
            slot: Some(1),
            target_dir: "/w/s/1".into(),
            how: "sticky".into(),
            refresh_secs: 0.4,
            refreshed_units: 12,
            ..Default::default()
        };
        assert_eq!(
            i.report_line(),
            "slot 1 (sticky) /w/s/1, 12 newer units from shared in 0.4s"
        );
    }
}
