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
}

impl SlotInfo {
    pub fn report_line(&self) -> String {
        match self.slot {
            Some(n) => {
                let mut s = format!("slot {n} ({}) {}", self.how, self.target_dir);
                if self.seed_secs > 0.0 {
                    s.push_str(&format!(", seeded in {:.1}s", self.seed_secs));
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

fn cp_reflink(srcs: &[PathBuf], dst: &Path) -> bool {
    if srcs.is_empty() {
        return true;
    }
    srcs.chunks(500).all(|chunk| {
        Command::new("cp")
            .arg("-a")
            .arg("--reflink=always")
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

fn cp_parallel(jobs: &[(PathBuf, PathBuf)]) -> bool {
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
                    if !cp_reflink(srcs, dst) {
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
        cp_parallel(&jobs)
    };
    stage(&[(".fingerprint", None)])
        && stage(&[
            ("build", None),
            ("deps", None),
            ("examples", None),
            ("incremental", Some(INCREMENTAL_MAX_AGE)),
        ])
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
    let metas: Vec<Option<SlotMeta>> = (0..cap)
        .map(|i| read_meta(&root.join(format!("{i}.json"))))
        .collect();
    let mut locks: Vec<Option<File>> = (0..cap).map(|_| None).collect();
    let (picked, busy) = pick(&metas, &me, |i| {
        locks[i] = try_lock(&root.join(format!("{i}.lock")));
        locks[i].is_some()
    });

    let Some(picked) = picked else {
        return Some(Slot {
            info: SlotInfo {
                slot: None,
                target_dir: target.to_string_lossy().into_owned(),
                how: "fallback".to_owned(),
                seed_secs: 0.0,
                busy,
                capacity: cap,
            },
            args: args.to_vec(),
            _lock: None,
        });
    };
    let (n, how) = match picked {
        Pick::Sticky(n) => (n, "sticky"),
        Pick::New(n) => (n, "new"),
        Pick::Reused(n) => (n, "reused"),
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
        });

    let mut seed_secs = 0.0;
    let slot_profile = slot_dir.join(&profile);
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
    }
    meta.owner = me;
    meta.last_used = paths::now();
    if let Ok(bytes) = serde_json::to_vec_pretty(&meta) {
        let _ = std::fs::write(&meta_path, bytes);
    }

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
        },
        args: new_args,
        _lock: lock,
    })
}

/// `justrust slots`: list the slots of the current workspace, or remove them.
pub fn command(clean: bool) -> anyhow::Result<()> {
    let target = shared_target_dir(&[])
        .ok_or_else(|| anyhow::anyhow!("not in a cargo workspace (cargo metadata failed)"))?;
    let root = target.join("justrust-slots");
    if !root.is_dir() {
        println!("no build slots under {}", target.display());
        return Ok(());
    }
    let mut n = 0;
    for i in 0..64 {
        let dir = root.join(i.to_string());
        let meta_path = root.join(format!("{i}.json"));
        if !dir.exists() && !meta_path.exists() {
            continue;
        }
        n += 1;
        let lock = try_lock(&root.join(format!("{i}.lock")));
        let in_use = lock.is_none();
        let meta = read_meta(&meta_path).unwrap_or_default();
        if clean {
            if in_use {
                println!("slot {i}: in use, kept");
                continue;
            }
            std::fs::remove_dir_all(&dir).ok();
            std::fs::remove_file(&meta_path).ok();
            println!("slot {i}: removed");
            continue;
        }
        let age = (paths::now() - meta.last_used).max(0.0);
        println!(
            "slot {i}: {}  owner {}  last used {} ago  seeded in {:.1}s  {}",
            if in_use { "IN USE" } else { "free  " },
            meta.owner,
            human_age(age),
            meta.seed_secs,
            dir.display()
        );
    }
    if n == 0 {
        println!("no build slots under {}", target.display());
    } else if !clean {
        println!(
            "slots share disk extents with {} until rebuilt (reflink copies). \
             `justrust slots --clean` removes free slots.",
            target.display()
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
    fn report_line_mentions_slot_and_seed() {
        let i = SlotInfo {
            slot: Some(2),
            target_dir: "/w/target/justrust-slots/2".into(),
            how: "new".into(),
            seed_secs: 12.5,
            busy: 1,
            capacity: 4,
        };
        assert_eq!(
            i.report_line(),
            "slot 2 (new) /w/target/justrust-slots/2, seeded in 12.5s"
        );
    }
}
