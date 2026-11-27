//! Garbage collection of rustc incremental compilation state, and
//! `justrust incr-gc`.
//!
//! A `-C incremental=<dir>` directory holds one crate directory per crate id
//! (`{crate}-{id}`, or `{crate}-{id}-{lineage}` with jr-cargo's stable local
//! ids). rustc collects old sessions inside the crate directory it compiles,
//! but nothing removes a crate directory that no build names any more: another
//! feature set, a renamed crate, an id scheme that changed with the toolchain.
//! On the hosted machine that was 49 GB on 2026-11-26, 46 GB of it under ids
//! the current toolchain no longer produces.
//!
//! What is removed, per incremental directory:
//! - crate directories unused for [`MAX_AGE_DAYS`],
//! - `{crate}-{id}` directories unused for [`SUPERSEDED_HOURS`] when a
//!   `{crate}-{id}-{lineage}` directory of the same crate was used since
//!   (the build moved to stable ids and will not read the old one again),
//! - then the least recently used ones until the directory fits [`MAX_GB`].
//!
//! Recency: every rustc session creates its lock file and session directory
//! in the crate directory, so the newest mtime of the directory and its
//! entries is its last use.
//!
//! Safety with concurrent builds:
//! - A directory used within [`MIN_IDLE_SECS`] is never removed, whatever the
//!   size.
//! - rustc holds a lock on `s-*.lock` while it writes a session (exclusive) or
//!   reads one (shared). The collector takes every lock of a crate directory
//!   exclusively without waiting and skips the directory when one is held. It
//!   keeps them while it deletes, which is what rustc's own collector does.
//! - A slot's private `justrust-slots/<n>/**/incremental` is only collected
//!   while the collector holds that slot's lease (`<n>.lock`), taken without
//!   waiting.
//! - It runs detached after a build, at most once per [`GC_INTERVAL_SECS`] and
//!   directory, at the lowest CPU priority. No build waits for it.
//!
//! Containment: a root must be a directory named `incremental`. Inside it only
//! direct children with a crate directory name are considered, and inside
//! those only `s-*` entries are deleted. Symlinks are never followed, and a
//! crate directory holding anything else is left in place.

use crate::paths;
use crate::sys::Detach;
use anyhow::Result;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// Crate directories unused this long are removed
/// (`JUSTRUST_INCR_GC_MAX_AGE_DAYS`).
const MAX_AGE_DAYS: f64 = 3.0;
/// Old-scheme directories replaced by a lineage directory are removed after
/// this long without use (`JUSTRUST_INCR_GC_SUPERSEDED_HOURS`).
const SUPERSEDED_HOURS: f64 = 6.0;
/// Size cap of one incremental directory (`JUSTRUST_INCR_GC_MAX_GB`, `0` = no
/// cap). The hosted stores were 6 to 8 GB of live state when this was set.
const MAX_GB: f64 = 10.0;
/// A directory used this recently is some build's warm state: never removed,
/// over the cap or not.
const MIN_IDLE_SECS: f64 = 3600.0;
/// At most one background collection per directory in this long.
const GC_INTERVAL_SECS: u64 = 3600;
/// Stamp and single-collector lock, inside the incremental directory. rustc
/// only looks at crate directories there.
const STAMP: &str = ".justrust-incr-gc";

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub max_age_secs: f64,
    pub superseded_secs: f64,
    /// 0 = no cap.
    pub max_bytes: u64,
    pub min_idle_secs: f64,
}

impl Policy {
    pub fn from_env() -> Policy {
        let f = |k: &str, d: f64| {
            std::env::var(k)
                .ok()
                .and_then(|v| v.parse::<f64>().ok())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .unwrap_or(d)
        };
        Policy {
            max_age_secs: f("JUSTRUST_INCR_GC_MAX_AGE_DAYS", MAX_AGE_DAYS) * 86400.0,
            superseded_secs: f("JUSTRUST_INCR_GC_SUPERSEDED_HOURS", SUPERSEDED_HOURS) * 3600.0,
            max_bytes: (f("JUSTRUST_INCR_GC_MAX_GB", MAX_GB) * 1e9) as u64,
            min_idle_secs: MIN_IDLE_SECS,
        }
    }
}

/// `{crate}-{id}` (stock ids) or `{crate}-{id}-{lineage}` (stable local ids).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scheme {
    Plain,
    Lineage,
}

/// The crate name and id scheme of a crate directory name, or None for
/// anything rustc did not name.
fn parse_crate_dir(name: &str) -> Option<(&str, Scheme)> {
    let mut parts = name.split('-');
    let krate = parts.next()?;
    let id = parts.next()?;
    let lineage = parts.next();
    if parts.next().is_some() {
        return None;
    }
    let alnum = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric());
    let ident = !krate.is_empty()
        && krate
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if !ident || !alnum(id) {
        return None;
    }
    match lineage {
        None => Some((krate, Scheme::Plain)),
        Some(l) if alnum(l) && l.len() <= 32 => Some((krate, Scheme::Lineage)),
        Some(_) => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
struct CrateDir {
    name: String,
    idle_secs: f64,
    bytes: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reason {
    Expired,
    Superseded,
    OverCap,
}

/// Which directories to remove and why. Pure: `dirs` in, indexes out.
fn plan(dirs: &[CrateDir], policy: &Policy) -> Vec<(usize, Reason)> {
    let parsed: Vec<Option<(&str, Scheme)>> =
        dirs.iter().map(|d| parse_crate_dir(&d.name)).collect();
    let mut out: Vec<(usize, Reason)> = Vec::new();
    let mut kept: Vec<usize> = Vec::new();
    for (i, d) in dirs.iter().enumerate() {
        let Some((krate, scheme)) = parsed[i] else {
            continue;
        };
        if d.idle_secs < policy.min_idle_secs {
            continue;
        }
        if d.idle_secs > policy.max_age_secs {
            out.push((i, Reason::Expired));
            continue;
        }
        let superseded = scheme == Scheme::Plain
            && d.idle_secs > policy.superseded_secs
            && dirs
                .iter()
                .zip(&parsed)
                .any(|(o, p)| *p == Some((krate, Scheme::Lineage)) && o.idle_secs < d.idle_secs);
        if superseded {
            out.push((i, Reason::Superseded));
        } else {
            kept.push(i);
        }
    }
    if policy.max_bytes > 0 {
        let removed: u64 = out.iter().map(|(i, _)| dirs[*i].bytes).sum();
        let mut used = dirs.iter().map(|d| d.bytes).sum::<u64>() - removed;
        kept.sort_by(|a, b| dirs[*b].idle_secs.total_cmp(&dirs[*a].idle_secs));
        for i in kept {
            if used <= policy.max_bytes {
                break;
            }
            used -= dirs[i].bytes;
            out.push((i, Reason::OverCap));
        }
    }
    out
}

fn idle_secs(now: SystemTime, t: SystemTime) -> f64 {
    now.duration_since(t).map_or(0.0, |d| d.as_secs_f64())
}

/// Last use of a crate directory: the newest mtime of it and its entries.
fn last_used(dir: &Path) -> Option<SystemTime> {
    let mut newest = std::fs::symlink_metadata(dir).ok()?.modified().ok()?;
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        if let Ok(t) = e.metadata().and_then(|m| m.modified()) {
            newest = newest.max(t);
        }
    }
    Some(newest)
}

/// Allocated bytes below `path`, hard links counted once, symlinks not
/// followed.
fn tree_bytes(path: &Path) -> u64 {
    fn walk(path: &Path, seen: &mut std::collections::HashSet<(u64, u64)>) -> u64 {
        let Ok(m) = std::fs::symlink_metadata(path) else {
            return 0;
        };
        #[cfg(unix)]
        let own = {
            use std::os::unix::fs::MetadataExt;
            if !m.is_dir() && m.nlink() > 1 && !seen.insert((m.dev(), m.ino())) {
                0
            } else {
                m.blocks() * 512
            }
        };
        #[cfg(not(unix))]
        let own = {
            let _ = &seen;
            m.len()
        };
        if !m.is_dir() {
            return own;
        }
        let below: u64 = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| walk(&e.path(), seen))
            .sum();
        own + below
    }
    walk(path, &mut std::collections::HashSet::new())
}

/// A root the collector accepts: an existing real directory named
/// `incremental`, canonicalized.
fn validated_root(root: &Path) -> Option<PathBuf> {
    let root = std::fs::canonicalize(root).ok()?;
    (root.file_name()? == "incremental" && root.is_dir()).then_some(root)
}

fn scan(root: &Path, now: SystemTime) -> Vec<CrateDir> {
    let mut dirs = Vec::new();
    for e in std::fs::read_dir(root).into_iter().flatten().flatten() {
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        // `file_type` does not follow symlinks.
        if parse_crate_dir(&name).is_none() || !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let path = e.path();
        let Some(used) = last_used(&path) else {
            continue;
        };
        dirs.push(CrateDir {
            name,
            idle_secs: idle_secs(now, used),
            bytes: tree_bytes(&path),
        });
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    dirs
}

/// A lock that is released when this is dropped, not when the last copy of
/// its descriptor closes. Closing is not enough: a child process started by
/// another thread inherits the descriptor until it execs, and the lock
/// belongs to the open file, so it outlived the drop. The next collection
/// then found its own stamp or a session lock taken (seen as a test that
/// removed nothing when the suite ran in parallel).
struct Held(File);

impl Drop for Held {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn try_lock_existing(path: &Path) -> std::io::Result<Option<Held>> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    match f.try_lock() {
        Ok(()) => Ok(Some(Held(f))),
        Err(std::fs::TryLockError::WouldBlock) => Ok(None),
        Err(std::fs::TryLockError::Error(e)) => Err(e),
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    Removed,
    /// A rustc session holds one of its locks.
    InUse,
    /// Used, or changed, since the scan.
    Changed,
    /// Sessions were deleted but something else is left in the directory, or
    /// a delete failed.
    Partial,
}

fn entry_names(dir: &Path) -> Option<Vec<String>> {
    let mut v: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    Some(v)
}

/// Remove the crate directory `name` of `root` unless a session holds it or
/// it was used since the scan. Only `s-*` entries are deleted.
fn remove_crate_dir(root: &Path, name: &str, policy: &Policy, now: SystemTime) -> Outcome {
    if parse_crate_dir(name).is_none() {
        return Outcome::Changed;
    }
    let dir = root.join(name);
    if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.is_dir()) {
        return Outcome::Changed;
    }
    let Some(before) = entry_names(&dir) else {
        return Outcome::Changed;
    };
    // Take every session lock. They are held until this function returns.
    let mut locks: Vec<Held> = Vec::new();
    for n in before.iter().filter(|n| n.ends_with(".lock")) {
        match try_lock_existing(&dir.join(n)) {
            Ok(Some(f)) => locks.push(f),
            Ok(None) => return Outcome::InUse,
            Err(_) => return Outcome::Changed,
        }
    }
    // With the locks held: a session that started after the scan has created
    // its own lock file and directory, which shows as a new entry or mtime.
    let fresh = last_used(&dir).is_none_or(|t| idle_secs(now, t) < policy.min_idle_secs);
    if fresh || entry_names(&dir).as_ref() != Some(&before) {
        return Outcome::Changed;
    }
    let mut clean = true;
    let sessions = before.iter().filter(|n| n.starts_with("s-"));
    // Session directories first, their lock files after, as rustc does.
    for lock_pass in [false, true] {
        for n in sessions.clone() {
            let p = dir.join(n);
            let Ok(m) = std::fs::symlink_metadata(&p) else {
                continue;
            };
            let r = match (m.is_dir(), lock_pass) {
                (true, false) => std::fs::remove_dir_all(&p),
                (false, true) => std::fs::remove_file(&p),
                _ => Ok(()),
            };
            clean &= r.is_ok();
        }
    }
    drop(locks);
    // Not recursive: fails, and leaves the directory, if anything is left.
    if clean && std::fs::remove_dir(&dir).is_ok() {
        Outcome::Removed
    } else {
        Outcome::Partial
    }
}

/// The lease file of the slot whose private target dir contains `root`
/// (`<slots>/<n>/...` -> `<slots>/<n>.lock`).
fn slot_lease_path(root: &Path) -> Option<PathBuf> {
    let comps: Vec<_> = root.components().collect();
    let at = comps
        .iter()
        .rposition(|c| c.as_os_str() == "justrust-slots")?;
    let n = comps.get(at + 1)?.as_os_str().to_str()?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let slots: PathBuf = comps[..=at].iter().collect();
    Some(slots.join(format!("{n}.lock")))
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct Report {
    pub root: String,
    pub dirs: usize,
    pub bytes_before: u64,
    pub removed: usize,
    pub bytes_freed: u64,
    pub expired: usize,
    pub superseded: usize,
    pub over_cap: usize,
    /// Skipped: a rustc session held a lock.
    pub in_use: usize,
    /// Skipped: used or changed between scan and removal.
    pub changed: usize,
    /// Not fully removed: foreign content or a failed delete.
    pub partial: usize,
    /// Nothing was removed because a build holds the slot's lease, or another
    /// collector is running here.
    pub skipped: Option<&'static str>,
    pub dry_run: bool,
    pub secs: f64,
}

impl Report {
    fn line(&self) -> String {
        let gb = |b: u64| format!("{:.2} GB", b as f64 / 1e9);
        let mut s = format!(
            "{}: {} dirs {} -> {} {}{} dirs {} (expired {}, superseded {}, over cap {})",
            self.root,
            self.dirs,
            gb(self.bytes_before),
            gb(self.bytes_before - self.bytes_freed),
            if self.dry_run {
                "would remove "
            } else {
                "removed "
            },
            self.removed,
            gb(self.bytes_freed),
            self.expired,
            self.superseded,
            self.over_cap,
        );
        if self.in_use + self.changed + self.partial > 0 {
            s.push_str(&format!(
                ", kept {} in use, {} just used, {} partial",
                self.in_use, self.changed, self.partial
            ));
        }
        if let Some(why) = self.skipped {
            s.push_str(&format!(", skipped: {why}"));
        }
        s
    }
}

/// Collect one incremental directory. Returns None when `root` is not one.
pub fn collect(root: &Path, policy: &Policy, dry_run: bool) -> Option<Report> {
    let t = std::time::Instant::now();
    let root = validated_root(root)?;
    let now = SystemTime::now();
    let mut rep = Report {
        root: root.display().to_string(),
        dry_run,
        ..Report::default()
    };
    // One collector per directory. The same file carries the hourly stamp.
    let stamp = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(root.join(STAMP))
        .ok();
    let Some(stamp) = stamp.filter(|f| f.try_lock().is_ok()).map(Held) else {
        rep.skipped = Some("another collector is running");
        return Some(rep);
    };
    if !dry_run {
        let _ = stamp.0.set_modified(now);
    }
    let dirs = scan(&root, now);
    rep.dirs = dirs.len();
    rep.bytes_before = dirs.iter().map(|d| d.bytes).sum();
    let todo = plan(&dirs, policy);
    // A slot's private directory belongs to whoever holds the slot.
    let _lease = match slot_lease_path(&root).filter(|_| !todo.is_empty() && !dry_run) {
        Some(lease) if lease.exists() => match try_lock_existing(&lease) {
            Ok(Some(f)) => Some(f),
            _ => {
                rep.skipped = Some("a build holds the slot");
                rep.secs = t.elapsed().as_secs_f64();
                return Some(rep);
            }
        },
        _ => None,
    };
    for (i, why) in todo {
        let d = &dirs[i];
        let outcome = if dry_run {
            Outcome::Removed
        } else {
            remove_crate_dir(&root, &d.name, policy, now)
        };
        match outcome {
            Outcome::Removed => {
                rep.removed += 1;
                rep.bytes_freed += d.bytes;
                match why {
                    Reason::Expired => rep.expired += 1,
                    Reason::Superseded => rep.superseded += 1,
                    Reason::OverCap => rep.over_cap += 1,
                }
            }
            Outcome::InUse => rep.in_use += 1,
            Outcome::Changed => rep.changed += 1,
            Outcome::Partial => rep.partial += 1,
        }
    }
    rep.secs = t.elapsed().as_secs_f64();
    Some(rep)
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

/// `<base>/<profile>/incremental` and `<base>/<triple>/<profile>/incremental`.
fn incremental_dirs_below(base: &Path, out: &mut Vec<PathBuf>) {
    for a in subdirs(base) {
        let name = a.file_name().map(|n| n.to_string_lossy().into_owned());
        if matches!(name.as_deref(), Some("justrust-slots" | "incremental")) {
            continue;
        }
        if a.join("incremental").is_dir() {
            out.push(a.join("incremental"));
            continue;
        }
        for b in subdirs(&a) {
            if b.join("incremental").is_dir() {
                out.push(b.join("incremental"));
            }
        }
    }
}

/// The incremental directories justrust points builds at: the hosted shared
/// stores (`~/.justrust/ws/<repo>.incr/<profile>/incremental`), and for a
/// target dir its own and its slots' (`justrust-slots/<n>/...`).
pub fn default_roots(target: Option<&Path>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(home) = paths::home() {
        for store in subdirs(&home.join("ws")) {
            if store.extension().is_some_and(|e| e == "incr") {
                for profile in subdirs(&store) {
                    if profile.join("incremental").is_dir() {
                        out.push(profile.join("incremental"));
                    }
                }
            }
        }
    }
    if let Some(target) = target {
        incremental_dirs_below(target, &mut out);
        for slot in subdirs(&target.join("justrust-slots")) {
            incremental_dirs_below(&slot, &mut out);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn due(root: &Path, now: SystemTime) -> bool {
    match std::fs::metadata(root.join(STAMP)).and_then(|m| m.modified()) {
        Ok(t) => now
            .duration_since(t)
            .is_ok_and(|d| d >= Duration::from_secs(GC_INTERVAL_SECS)),
        Err(_) => true,
    }
}

/// The target dir shared by a run's slots: `<target>/justrust-slots/<n>` ->
/// `<target>`.
pub fn target_base(target_dir: &Path, in_slot: bool) -> PathBuf {
    match (in_slot, target_dir.parent().and_then(Path::parent)) {
        (true, Some(base)) => base.to_path_buf(),
        _ => target_dir.to_path_buf(),
    }
}

/// After a run: start a detached collection of the incremental directories
/// that had none in the last hour. Never blocks or fails the build.
pub fn maybe_background(target: Option<&Path>) {
    // In unit tests `current_exe` is the test binary.
    if cfg!(test) || std::env::var("JUSTRUST_INCR_GC").as_deref() == Ok("0") {
        return;
    }
    let now = SystemTime::now();
    let roots: Vec<PathBuf> = default_roots(target)
        .into_iter()
        .filter(|r| due(r, now))
        .collect();
    if roots.is_empty() {
        return;
    }
    // Claim the hour before spawning so concurrent runs start one collector.
    for r in &roots {
        let _ = std::fs::write(r.join(STAMP), b"");
    }
    let Ok(me) = std::env::current_exe() else {
        return;
    };
    let _ = std::process::Command::new(me)
        .arg("incr-gc")
        .arg("--background")
        .args(&roots)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .detached()
        .spawn();
}

/// `justrust incr-gc`: collect `roots`, or the default ones of the current
/// workspace and this machine's shared stores.
pub fn command(roots: Vec<PathBuf>, dry_run: bool, background: bool) -> Result<()> {
    if background {
        // Stay out of the way of whatever build runs next.
        #[cfg(unix)]
        unsafe {
            libc::setpriority(libc::PRIO_PROCESS, 0, 19);
        }
    }
    let roots = if roots.is_empty() {
        let target = std::env::var_os("CARGO_TARGET_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok().map(|d| d.join("target")));
        default_roots(target.as_deref())
    } else {
        roots
    };
    let policy = Policy::from_env();
    let (mut before, mut freed) = (0u64, 0u64);
    for root in &roots {
        let Some(rep) = collect(root, &policy, dry_run) else {
            if !background {
                println!(
                    "{}: not an incremental directory, left alone",
                    root.display()
                );
            }
            continue;
        };
        before += rep.bytes_before;
        freed += rep.bytes_freed;
        if background {
            if rep.removed + rep.in_use + rep.partial > 0
                && let Ok(home) = paths::home()
                && let Ok(line) = serde_json::to_string(&rep)
            {
                let _ = paths::append_line(&home.join("incr-gc.log"), &line);
            }
        } else {
            println!("{}", rep.line());
        }
    }
    if !background {
        println!(
            "total: {:.2} GB -> {:.2} GB in {} incremental directories{}",
            before as f64 / 1e9,
            (before - freed) as f64 / 1e9,
            roots.len(),
            if dry_run { " (dry run)" } else { "" }
        );
    }
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const HOUR: f64 = 3600.0;

    fn policy() -> Policy {
        Policy {
            max_age_secs: 72.0 * HOUR,
            superseded_secs: 6.0 * HOUR,
            max_bytes: 0,
            min_idle_secs: HOUR,
        }
    }

    struct Tmp(PathBuf);
    impl Tmp {
        /// `<tmp>/<name>-<pid>/<rel>` created, `rel` ending in `incremental`.
        fn new(name: &str, rel: &str) -> (Tmp, PathBuf) {
            let base = std::env::temp_dir()
                .join(format!("justrust-incr-gc-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&base);
            let root = base.join(rel);
            std::fs::create_dir_all(&root).unwrap();
            (Tmp(base), root)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A crate directory with one finalized session, last used `hours` ago.
    fn crate_dir(root: &Path, name: &str, hours: f64) -> PathBuf {
        let dir = root.join(name);
        let session = dir.join("s-hojaa3ezlm-1ux6ad6-bcpys13h8v86");
        std::fs::create_dir_all(&session).unwrap();
        std::fs::write(session.join("dep-graph.bin"), vec![7u8; 8192]).unwrap();
        std::fs::write(session.join("query-cache.bin"), b"q").unwrap();
        std::fs::write(dir.join("s-hojaa3ezlm-1ux6ad6.lock"), b"").unwrap();
        age(&dir, hours);
        dir
    }

    fn age(dir: &Path, hours: f64) {
        let t = SystemTime::now() - Duration::from_secs_f64(hours * HOUR);
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            File::open(e.path()).unwrap().set_modified(t).unwrap();
        }
        File::open(dir).unwrap().set_modified(t).unwrap();
    }

    fn d(name: &str, idle_hours: f64, bytes: u64) -> CrateDir {
        CrateDir {
            name: name.into(),
            idle_secs: idle_hours * HOUR,
            bytes,
        }
    }

    #[test]
    fn crate_dir_names_parse_strictly() {
        assert_eq!(
            parse_crate_dir("justrust-0ioh3i951v73p"),
            Some(("justrust", Scheme::Plain))
        );
        assert_eq!(
            parse_crate_dir("jcode_base-0ioh3i951v73p-4b877f25ea71ffce"),
            Some(("jcode_base", Scheme::Lineage))
        );
        for bad in [
            "",
            "justrust",
            "-abc",
            "a-",
            "a-b-",
            "a-b-c-d",
            "a b-c",
            "..-x",
            ".trash-1",
            ".justrust-incr-gc",
            "a-b/c",
            "a-b-../x",
        ] {
            assert_eq!(parse_crate_dir(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn plan_removes_expired_and_superseded_but_nothing_recent() {
        let dirs = [
            d("a-old1", 100.0, 1),           // expired
            d("a-old2", 10.0, 1),            // superseded by a-new-l1
            d("a-new-l1", 2.0, 1),           // current
            d("b-old", 10.0, 1),             // no lineage sibling: kept
            d("c-old", 3.0, 1),              // sibling, but inside the grace
            d("c-new-l1", 0.1, 1),           //
            d("e-old", 8.0, 1),              // lineage sibling is older: kept
            d("e-new-l1", 9.0, 1),           //
            d("f-old", 0.5, 1 << 40),        // used in the last hour
            d("not a crate dir", 1000.0, 1), // never ours
        ];
        let got = plan(&dirs, &policy());
        assert_eq!(got, vec![(0, Reason::Expired), (1, Reason::Superseded)]);
    }

    #[test]
    fn plan_trims_least_recently_used_to_the_cap() {
        let dirs = [
            d("a-1", 2.0, 40),
            d("b-1", 50.0, 40),
            d("c-1", 20.0, 40),
            d("d-1", 0.2, 40), // recent: counts, never removed
            d("e-1", 0.1, 40),
        ];
        let p = Policy {
            max_bytes: 100,
            ..policy()
        };
        // 200 -> remove b (160), c (120), a (80).
        assert_eq!(
            plan(&dirs, &p),
            vec![
                (1, Reason::OverCap),
                (2, Reason::OverCap),
                (0, Reason::OverCap)
            ]
        );
        // The cap cannot be met from recent directories alone.
        let p = Policy {
            max_bytes: 10,
            ..policy()
        };
        assert_eq!(plan(&dirs, &p).len(), 3);
        // Under the cap: nothing.
        let p = Policy {
            max_bytes: 200,
            ..policy()
        };
        assert!(plan(&dirs, &p).is_empty());
    }

    #[test]
    fn collects_stale_directories_and_only_those() {
        let (_tmp, root) = Tmp::new("basic", "debug/incremental");
        let stale = crate_dir(&root, "app-0aaaaaaaaaaaa", 100.0);
        let superseded = crate_dir(&root, "lib-0bbbbbbbbbbbb", 10.0);
        let current = crate_dir(&root, "lib-0ccccccccccccc-1a2b3c4d", 2.0);
        let recent = crate_dir(&root, "app-0ddddddddddddd", 0.0);
        // Things rustc did not put there are not ours, however old.
        let foreign = root.join("my notes");
        std::fs::create_dir_all(&foreign).unwrap();
        std::fs::write(foreign.join("x"), b"x").unwrap();
        age(&foreign, 1000.0);
        std::fs::write(root.join("README"), b"x").unwrap();

        let dry = collect(&root, &policy(), true).unwrap();
        assert_eq!((dry.dirs, dry.removed), (4, 2));
        assert!(
            stale.exists() && superseded.exists(),
            "dry run deletes nothing"
        );

        let rep = collect(&root, &policy(), false).unwrap();
        assert_eq!((rep.removed, rep.expired, rep.superseded), (2, 1, 1));
        assert!(rep.bytes_freed >= 2 * 8192 && rep.bytes_freed <= rep.bytes_before);
        assert!(!stale.exists() && !superseded.exists());
        assert!(
            current
                .join("s-hojaa3ezlm-1ux6ad6-bcpys13h8v86/dep-graph.bin")
                .exists()
        );
        assert!(recent.exists() && foreign.join("x").exists() && root.join("README").exists());
        // Idempotent.
        let again = collect(&root, &policy(), false).unwrap();
        assert_eq!((again.dirs, again.removed), (2, 0));
    }

    /// A child started by another thread holds a copy of every open
    /// descriptor until it execs. A lock the collector dropped must be free
    /// at once anyway.
    #[cfg(unix)]
    #[test]
    fn a_dropped_lock_is_free_while_other_threads_start_processes() {
        let (_tmp, root) = Tmp::new("fork", "debug/incremental");
        let path = root.join("x.lock");
        std::fs::write(&path, b"").unwrap();
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let spawners: Vec<_> = (0..4)
            .map(|_| {
                let stop = stop.clone();
                std::thread::spawn(move || {
                    while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                        let _ = std::process::Command::new("/bin/true").status();
                    }
                })
            })
            .collect();
        let mut busy = 0;
        for _ in 0..3000 {
            drop(try_lock_existing(&path).unwrap());
            busy += usize::from(try_lock_existing(&path).unwrap().is_none());
        }
        stop.store(true, std::sync::atomic::Ordering::Relaxed);
        for s in spawners {
            s.join().unwrap();
        }
        assert_eq!(busy, 0);
    }

    #[test]
    fn a_directory_a_session_holds_is_left_alone() {
        let (_tmp, root) = Tmp::new("inuse", "debug/incremental");
        let held = crate_dir(&root, "app-0aaaaaaaaaaaa", 100.0);
        let free = crate_dir(&root, "lib-0bbbbbbbbbbbb", 100.0);
        // What rustc does while it reads a session as its source: a shared
        // lock on the session's lock file.
        let lock = File::open(held.join("s-hojaa3ezlm-1ux6ad6.lock")).unwrap();
        lock.try_lock_shared().unwrap();

        let rep = collect(&root, &policy(), false).unwrap();
        assert_eq!((rep.removed, rep.in_use), (1, 1));
        assert!(!free.exists());
        assert!(
            held.join("s-hojaa3ezlm-1ux6ad6-bcpys13h8v86/dep-graph.bin")
                .exists()
        );
        assert!(held.join("s-hojaa3ezlm-1ux6ad6.lock").exists());

        // A writer (exclusive lock on a second, working session) too.
        lock.unlock().unwrap();
        let working = held.join("s-hojzzzzzzz-0000001.lock");
        std::fs::write(&working, b"").unwrap();
        age(&held, 100.0);
        let writer = File::open(&working).unwrap();
        writer.try_lock().unwrap();
        let rep = collect(&root, &policy(), false).unwrap();
        assert_eq!((rep.removed, rep.in_use), (0, 1));
        assert!(held.join("s-hojaa3ezlm-1ux6ad6-bcpys13h8v86").exists());

        // Released: collected.
        writer.unlock().unwrap();
        let rep = collect(&root, &policy(), false).unwrap();
        assert_eq!((rep.removed, rep.in_use), (1, 0));
        assert!(!held.exists());
    }

    #[test]
    fn a_directory_used_since_the_scan_is_left_alone() {
        let (_tmp, root) = Tmp::new("raced", "debug/incremental");
        let dir = crate_dir(&root, "app-0aaaaaaaaaaaa", 100.0);
        // A session that started after the scan: new lock file and directory.
        std::fs::write(dir.join("s-hojzzzzzzz-0000001.lock"), b"").unwrap();
        std::fs::create_dir(dir.join("s-hojzzzzzzz-0000001-working")).unwrap();
        let out = remove_crate_dir(&root, "app-0aaaaaaaaaaaa", &policy(), SystemTime::now());
        assert_eq!(out, Outcome::Changed);
        assert!(
            dir.join("s-hojaa3ezlm-1ux6ad6-bcpys13h8v86/dep-graph.bin")
                .exists()
        );
    }

    #[test]
    fn a_leased_slot_keeps_its_private_directory() {
        let (tmp, root) = Tmp::new("slot", "target/justrust-slots/3/debug/incremental");
        let dir = crate_dir(&root, "app-0aaaaaaaaaaaa", 100.0);
        let lease_path = tmp.0.join("target/justrust-slots/3.lock");
        assert_eq!(
            slot_lease_path(&validated_root(&root).unwrap()),
            Some(
                std::fs::canonicalize(tmp.0.join("target/justrust-slots"))
                    .unwrap()
                    .join("3.lock")
            )
        );
        std::fs::write(&lease_path, b"").unwrap();
        let lease = File::open(&lease_path).unwrap();
        lease.try_lock().unwrap();

        let rep = collect(&root, &policy(), false).unwrap();
        assert_eq!(rep.removed, 0);
        assert_eq!(rep.skipped, Some("a build holds the slot"));
        assert!(dir.exists());

        lease.unlock().unwrap();
        let rep = collect(&root, &policy(), false).unwrap();
        assert_eq!((rep.removed, rep.skipped), (1, None));
        assert!(!dir.exists());
        // The shared directory next to the slots has no lease.
        assert_eq!(
            slot_lease_path(Path::new("/w/target/debug/incremental")),
            None
        );
        assert_eq!(
            slot_lease_path(Path::new("/w/target/justrust-slots/x/debug/incremental")),
            None
        );
    }

    #[test]
    fn deletes_nothing_outside_what_it_was_given() {
        let (tmp, root) = Tmp::new("contain", "debug/incremental");
        // Not named `incremental`: refused outright.
        let deps = tmp.0.join("debug/deps");
        crate_dir(&deps, "app-0aaaaaaaaaaaa", 100.0);
        assert!(collect(&deps, &policy(), false).is_none());
        assert!(collect(&tmp.0.join("missing/incremental"), &policy(), false).is_none());
        assert!(deps.join("app-0aaaaaaaaaaaa").exists());

        // A symlink with a crate directory name is not followed.
        let outside = crate_dir(&tmp.0.join("elsewhere"), "app-0bbbbbbbbbbbb", 100.0);
        std::os::unix::fs::symlink(&outside, root.join("app-0bbbbbbbbbbbb")).unwrap();
        // Files that are not session state survive with their directory.
        let dir = crate_dir(&root, "app-0ccccccccccccc", 100.0);
        std::fs::write(dir.join("keep.txt"), b"mine").unwrap();
        age(&dir, 100.0);

        let rep = collect(&root, &policy(), false).unwrap();
        assert_eq!((rep.dirs, rep.removed, rep.partial), (1, 0, 1));
        assert_eq!(std::fs::read(dir.join("keep.txt")).unwrap(), b"mine");
        assert!(root.join("app-0bbbbbbbbbbbb").is_symlink());

        // A symlink named like a session is unlinked, never followed.
        let linked = crate_dir(&root, "app-0ddddddddddddd", 100.0);
        std::os::unix::fs::symlink(&outside, linked.join("s-link")).unwrap();
        let p = Policy {
            min_idle_secs: 0.0,
            ..policy()
        };
        let out = remove_crate_dir(&root, "app-0ddddddddddddd", &p, SystemTime::now());
        assert_eq!(out, Outcome::Removed);
        assert!(
            outside
                .join("s-hojaa3ezlm-1ux6ad6-bcpys13h8v86/dep-graph.bin")
                .exists()
        );
    }

    #[test]
    fn finds_shared_slot_and_triple_directories_and_throttles() {
        let (tmp, shared) = Tmp::new("roots", "target/debug/incremental");
        let target = tmp.0.join("target");
        let triple = target.join("x86_64-unknown-linux-gnu/release/incremental");
        let slot = target.join("justrust-slots/0/debug/incremental");
        std::fs::create_dir_all(&triple).unwrap();
        std::fs::create_dir_all(&slot).unwrap();
        std::fs::create_dir_all(target.join("debug/deps/incremental")).unwrap();
        let mut found = Vec::new();
        incremental_dirs_below(&target, &mut found);
        for s in subdirs(&target.join("justrust-slots")) {
            incremental_dirs_below(&s, &mut found);
        }
        found.sort();
        assert_eq!(found, vec![shared.clone(), slot.clone(), triple]);
        assert_eq!(target_base(&target.join("justrust-slots/0"), true), target);
        assert_eq!(target_base(&target, false), target);

        let now = SystemTime::now();
        assert!(due(&shared, now));
        collect(&shared, &policy(), false).unwrap();
        assert!(!due(&shared, now));
        assert!(due(
            &shared,
            now + Duration::from_secs(GC_INTERVAL_SECS + 1)
        ));
    }
}
