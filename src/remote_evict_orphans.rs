//! Second GC pass: cargo build directories nobody registered.
//!
//! Registered roots only cover mirrors enrolled since the registry existed.
//! Older mirrors, scratch copies built with a custom `CARGO_TARGET_DIR`, and
//! ad hoc target dirs under `~/.cache` filled the hosted disk on 2026-11-08
//! (about 130 GB) while `remote gc` reported nothing to do. This pass finds
//! them by cargo's own marker and deletes only those that are clearly idle:
//!
//! - Identity: a real directory (no symlinks) containing a `CACHEDIR.TAG`
//!   written by cargo, owned by this user, on the scan base's filesystem.
//! - Scope: scanned below the parents of registered roots, `~/.cache` and
//!   `~/.jcode/scratch`; never inside `.justrust`, `.cargo`, `.rustup`,
//!   `.git`, or a registered root (the first pass owns those).
//! - Idle: nothing in its top two levels or `*/.fingerprint` changed within
//!   `ORPHAN_IDLE`, and no other process has a cwd or open file below its
//!   parent. Both are rechecked right before deletion under the parent's
//!   eviction lock.
//!
//! `~/.justrust/sent` (delta bases for artifact downloads) is also pruned by
//! age: a missing base only means the next download is a full one.
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::Probe;

/// An unregistered build dir untouched this long is deleted under pressure.
pub const ORPHAN_IDLE: Duration = Duration::from_secs(6 * 3600);
/// Delta bases unused this long are dropped on every pass.
pub const SENT_MAX_AGE: Duration = Duration::from_secs(3 * 24 * 3600);
/// Local run logs older than this are pruned on hosted machines (newest `RUNS_KEEP_NEWEST` always kept).
pub const RUNS_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
pub const RUNS_KEEP_NEWEST: usize = 200;
/// Stale user-owned build temp entries in `/tmp` older than this are pruned.
pub const TMP_MAX_AGE: Duration = Duration::from_secs(24 * 3600);
/// Unreferenced `jr-*` toolchains older than this are pruned on hosted machines.
pub const TOOLCHAIN_MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
/// Half-written snapshots older than this are leftovers of a dead stream.
const SENT_TEMP_AGE: Duration = Duration::from_secs(3600);
const MAX_DEPTH: usize = 7;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Orphan {
    pub dir: PathBuf,
    pub bytes: u64,
    pub last_change: u64,
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn mtime(p: &Path) -> u64 {
    std::fs::symlink_metadata(p)
        .and_then(|m| m.modified())
        .map_or(0, secs)
}

/// A directory cargo created as a build directory.
pub fn is_cargo_build_dir(dir: &Path) -> bool {
    std::fs::read_to_string(dir.join("CACHEDIR.TAG")).is_ok_and(|s| {
        s.starts_with("Signature: 8a477f597d28d172789f06886806bc55") && s.contains("cargo")
    })
}

/// Newest change in the parts of a build dir every build writes.
pub fn last_change(dir: &Path) -> u64 {
    let mut newest = mtime(dir);
    let Ok(rd) = std::fs::read_dir(dir) else {
        return newest;
    };
    for e in rd.flatten() {
        let p = e.path();
        newest = newest.max(mtime(&p));
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            if let Ok(rd) = std::fs::read_dir(&p) {
                for e in rd.flatten() {
                    newest = newest.max(mtime(&e.path()));
                }
            }
            newest = newest.max(mtime(&p.join(".fingerprint")));
        }
    }
    newest
}

fn excluded(name: &str) -> bool {
    matches!(
        name,
        ".justrust" | ".cargo" | ".rustup" | ".git" | "node_modules" | "proc" | "sys"
    )
}

/// Every cargo build dir below `bases`, skipping registered roots.
pub fn find(bases: &[PathBuf], registered: &[PathBuf]) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, dev: u64, registered: &[PathBuf], out: &mut Vec<PathBuf>) {
        use std::os::unix::fs::MetadataExt;
        if registered.iter().any(|r| r == dir) {
            return;
        }
        if is_cargo_build_dir(dir) {
            out.push(dir.to_path_buf());
            return;
        }
        if depth == 0 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let Ok(t) = e.file_type() else { continue };
            if !t.is_dir() || t.is_symlink() {
                continue;
            }
            if e.file_name().to_str().is_none_or(excluded) {
                continue;
            }
            let p = e.path();
            if std::fs::symlink_metadata(&p).is_ok_and(|m| m.dev() == dev) {
                walk(&p, depth - 1, dev, registered, out);
            }
        }
    }
    let mut out = Vec::new();
    for base in bases {
        use std::os::unix::fs::MetadataExt;
        if base
            .components()
            .any(|c| c.as_os_str().to_str().is_none_or(excluded))
        {
            continue;
        }
        let Ok(m) = std::fs::symlink_metadata(base) else {
            continue;
        };
        if m.is_dir() && !m.file_type().is_symlink() {
            walk(base, MAX_DEPTH, m.dev(), registered, &mut out);
        }
    }
    out.sort();
    out.dedup();
    // A build dir nested in another one is covered by the outer one.
    let mut kept: Vec<PathBuf> = Vec::new();
    for d in out {
        if !kept.iter().any(|k| d.starts_with(k)) {
            kept.push(d);
        }
    }
    kept
}

/// The path is still exactly the build dir that was planned: canonical,
/// owned by us, cargo-tagged.
pub fn still_safe(dir: &Path) -> Result<(), String> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(dir).map_err(|e| e.to_string())?;
    if m.file_type().is_symlink() || !m.is_dir() {
        return Err("not a real directory".into());
    }
    if dir.canonicalize().map_err(|e| e.to_string())? != dir {
        return Err("path goes through a symlink".into());
    }
    if m.uid() != unsafe { libc::geteuid() } {
        return Err("belongs to another principal".into());
    }
    if !is_cargo_build_dir(dir) {
        return Err("no cargo CACHEDIR.TAG".into());
    }
    if dir
        .components()
        .any(|c| c.as_os_str().to_str().is_none_or(excluded))
    {
        return Err("inside a private/cache tree".into());
    }
    Ok(())
}

/// Idle orphans, oldest first, until `need` bytes are covered.
pub fn plan(dirs: &[PathBuf], need: u64, idle: Duration, probe: &dyn Probe) -> Vec<Orphan> {
    let now = probe.now();
    let mut c: Vec<Orphan> = dirs
        .iter()
        .filter(|d| still_safe(d).is_ok())
        .filter_map(|d| {
            let last_change = last_change(d);
            if last_change.saturating_add(idle.as_secs()) > now {
                return None;
            }
            if probe.in_use_by_other(&guard(d)) {
                return None;
            }
            Some(Orphan {
                dir: d.clone(),
                bytes: probe.size(d),
                last_change,
            })
        })
        .collect();
    c.sort_by_key(|o| (o.last_change, std::cmp::Reverse(o.bytes)));
    let mut freed = 0u64;
    c.retain(|o| {
        let keep = freed < need;
        if keep {
            freed = freed.saturating_add(o.bytes);
        }
        keep
    });
    c
}

/// What must be unused: the project owning the build dir if its parent is
/// a cargo project, else the build dir alone (a shared scratch dir holding
/// many `X-target` dirs is always in use by something).
fn guard(dir: &Path) -> PathBuf {
    match dir.parent() {
        Some(p) if p.join("Cargo.toml").is_file() => p.to_path_buf(),
        _ => dir.to_path_buf(),
    }
}

/// Delete one planned orphan after rechecking under the parent's lock.
pub fn evict(o: &Orphan, idle: Duration, probe: &dyn Probe) -> std::io::Result<()> {
    let parent = o
        .dir
        .parent()
        .ok_or_else(|| std::io::Error::other("no parent"))?;
    let _lock = super::evict_lock(parent)?;
    still_safe(&o.dir).map_err(std::io::Error::other)?;
    if last_change(&o.dir).saturating_add(idle.as_secs()) > probe.now() {
        return Err(std::io::Error::other("changed since planning"));
    }
    if probe.in_use_by_other(&guard(&o.dir)) {
        return Err(std::io::Error::other("in use"));
    }
    std::fs::remove_dir_all(&o.dir)
}

/// Prune delta bases older than `max_age` (and their ids), plus stale temp
/// files. Returns (files removed, bytes).
pub fn sweep_sent(dir: &Path, max_age: Duration, now: SystemTime) -> (usize, u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (0, 0);
    };
    let (mut n, mut bytes) = (0, 0);
    for e in rd.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() {
            continue;
        }
        let name = e.file_name();
        let name = name.to_string_lossy();
        let temp = name.ends_with(".new") || name.ends_with(".zst");
        let limit = if temp { SENT_TEMP_AGE } else { max_age };
        // Ids are judged by their base. The base is renamed into place before
        // its id is written, so an id without a base is dead.
        let expired = if let Some(base) = name.strip_suffix(".id") {
            match std::fs::metadata(dir.join(base)) {
                Ok(b) => older(&b, now, max_age),
                Err(_) => true,
            }
        } else {
            older(&m, now, limit)
        };
        if expired && std::fs::remove_file(e.path()).is_ok() {
            n += 1;
            bytes += m.len();
        }
    }
    (n, bytes)
}

fn older(m: &std::fs::Metadata, now: SystemTime, limit: Duration) -> bool {
    m.modified()
        .ok()
        .and_then(|t| now.duration_since(t).ok())
        .is_some_and(|age| age > limit)
}

/// Prune old `~/.justrust/runs/<id>` directories beyond `keep_newest` that are
/// older than `max_age`. Never follows symlinks. Returns count removed.
pub fn sweep_runs(dir: &Path, max_age: Duration, keep_newest: usize, now: SystemTime) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut entries: Vec<(u64, PathBuf, std::fs::Metadata)> = Vec::new();
    for e in rd.flatten() {
        let path = e.path();
        let Ok(m) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !m.is_dir() || m.file_type().is_symlink() {
            continue;
        }
        let t = m.modified().map_or(0, secs);
        entries.push((t, path, m));
    }
    if entries.len() <= keep_newest {
        return 0;
    }
    // Newest first so the top `keep_newest` are always spared.
    entries.sort_by_key(|a| std::cmp::Reverse(a.0));
    let mut removed = 0;
    for (_, path, m) in entries.into_iter().skip(keep_newest) {
        if older(&m, now, max_age) && std::fs::remove_dir_all(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

fn is_build_tmp_name(name: &str) -> bool {
    name.starts_with("justrust-")
        || name.starts_with("jr-")
        || name.starts_with("rustc")
        || name.starts_with("cargo-")
        || name.starts_with("rustdoc")
}

/// Prune stale user-owned build temp files/dirs in `tmp_dir` (e.g. `/tmp`)
/// older than `max_age` and not currently used by any process.
pub fn sweep_tmp(dir: &Path, max_age: Duration, now: SystemTime, probe: &dyn Probe) -> usize {
    use std::os::unix::fs::MetadataExt;
    let uid = unsafe { libc::geteuid() };
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for e in rd.flatten() {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        if !is_build_tmp_name(name) {
            continue;
        }
        let path = e.path();
        let Ok(m) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if m.file_type().is_symlink() || m.uid() != uid {
            continue;
        }
        if !older(&m, now, max_age) || probe.in_use_by_other(&path) {
            continue;
        }
        let ok = if m.is_dir() {
            std::fs::remove_dir_all(&path).is_ok()
        } else {
            std::fs::remove_file(&path).is_ok()
        };
        if ok {
            removed += 1;
        }
    }
    removed
}

/// Extract referenced toolchain IDs from `.slots.json` and `toolchain-policy.json`.
/// Returns `None` if `.slots.json` exists but cannot be parsed (fail closed).
fn protected_toolchains(justrust_home: &Path) -> Option<std::collections::HashSet<String>> {
    let mut keep = std::collections::HashSet::new();
    let slots_path = justrust_home.join("toolchains/.slots.json");
    match std::fs::read(&slots_path) {
        Ok(bytes) => {
            let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
            for ptr in ["/active/id", "/last_good", "/previous/id"] {
                if let Some(id) = v.pointer(ptr).and_then(|s| s.as_str()) {
                    keep.insert(id.to_owned());
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => return None,
    }
    let policy_path = justrust_home.join("toolchain-policy.json");
    if let Ok(bytes) = std::fs::read(&policy_path)
        && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && let Some(id) = v.get("toolchain").and_then(|s| s.as_str())
    {
        keep.insert(id.to_owned());
    }
    Some(keep)
}

fn try_lock_usage(path: &Path) -> Result<Option<std::fs::File>, ()> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::OpenOptionsExt;
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)
    {
        Ok(f) => {
            // SAFETY: owned open descriptor; unlocked on drop.
            if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
                Ok(Some(f))
            } else {
                Err(())
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(_) => Err(()),
    }
}

/// Prune unreferenced, inactive `jr-*` toolchains under `<home>/toolchains/`
/// and `<home>/toolchains/staged/` older than `max_age`. Never touches
/// `managed/`, non-`jr-*` channels, or anything referenced by `.slots.json` or
/// `toolchain-policy.json`.
pub fn sweep_toolchains(
    justrust_home: &Path,
    max_age: Duration,
    now: SystemTime,
    probe: &dyn Probe,
) -> usize {
    let Some(protected) = protected_toolchains(justrust_home) else {
        return 0;
    };
    let root = justrust_home.join("toolchains");
    let mut removed = 0;
    for parent in [root.clone(), root.join("staged")] {
        let Ok(rd) = std::fs::read_dir(&parent) else {
            continue;
        };
        for e in rd.flatten() {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            if !name.starts_with("jr-")
                || !crate::toolchain::Spec::parse(name).is_ok_and(|s| s.is_jr() && s.id() == name)
                || protected.contains(name)
            {
                continue;
            }
            let path = e.path();
            let Ok(m) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if !m.is_dir() || m.file_type().is_symlink() || !older(&m, now, max_age) {
                continue;
            }
            let usage_lock = root.join(format!(".usage-{name}.lock"));
            if let Ok(um) = std::fs::symlink_metadata(&usage_lock)
                && !older(&um, now, max_age)
            {
                continue;
            }
            let Ok(_guard) = try_lock_usage(&usage_lock) else {
                continue;
            };
            if probe.in_use_by_other(&path) {
                continue;
            }
            if std::fs::remove_dir_all(&path).is_ok() {
                removed += 1;
            }
        }
    }
    removed
}

/// Where orphans may live: parents of registered roots, `~/.cache`,
/// `~/.jcode/scratch`.
pub fn bases(home: &Path, registered: &[PathBuf]) -> Vec<PathBuf> {
    let mut b: Vec<PathBuf> = registered
        .iter()
        .filter_map(|r| r.ancestors().nth(1).map(Path::to_path_buf))
        .collect();
    b.push(home.join(".cache"));
    b.push(home.join(".jcode/scratch"));
    b.sort();
    b.dedup();
    let mut kept: Vec<PathBuf> = Vec::new();
    for d in b {
        if !kept.iter().any(|k| d.starts_with(k)) {
            kept.push(d);
        }
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    const TAG: &str = "Signature: 8a477f597d28d172789f06886806bc55\n# This file is a cache directory tag created by cargo.\n";

    struct P(u64, Vec<PathBuf>);
    impl Probe for P {
        fn disk(&self, _: &Path) -> Option<(u64, u64)> {
            None
        }
        fn size(&self, _: &Path) -> u64 {
            10
        }
        fn in_use_by_other(&self, p: &Path) -> bool {
            self.1.iter().any(|b| b.starts_with(p))
        }
        fn now(&self) -> u64 {
            self.0
        }
    }

    fn build_dir(p: &Path) {
        std::fs::create_dir_all(p.join("debug/.fingerprint")).unwrap();
        std::fs::write(p.join("CACHEDIR.TAG"), TAG).unwrap();
    }

    #[test]
    fn finds_tagged_dirs_and_respects_exclusions() {
        let base = super::super::tests::scratch("orphan-find");
        build_dir(&base.join("a/target"));
        build_dir(&base.join("b/custom-target"));
        build_dir(&base.join("c/.justrust/target"));
        build_dir(&base.join("reg/target"));
        build_dir(&base.join("a/target/nested/target"));
        std::fs::create_dir_all(base.join("plain/target")).unwrap();
        std::fs::write(base.join("fake-tag/CACHEDIR.TAG"), "x").ok();
        std::fs::create_dir_all(base.join("fake")).unwrap();
        std::fs::write(base.join("fake/CACHEDIR.TAG"), "Signature: nope").unwrap();
        std::os::unix::fs::symlink(base.join("a"), base.join("link")).unwrap();
        let found = find(std::slice::from_ref(&base), &[base.join("reg/target")]);
        assert_eq!(
            found,
            vec![base.join("a/target"), base.join("b/custom-target")]
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn only_idle_unused_dirs_are_planned_and_deleted() {
        let base = super::super::tests::scratch("orphan-plan");
        let (old, busy) = (base.join("old/target"), base.join("busy/target"));
        build_dir(&old);
        build_dir(&busy);
        std::fs::write(base.join("old/src.rs"), "keep").unwrap();
        std::fs::write(base.join("busy/Cargo.toml"), "").unwrap();
        let loose = base.join("loose-target");
        build_dir(&loose);
        let now = secs(SystemTime::now());
        let dirs = vec![old.clone(), busy.clone(), loose.clone()];
        let fresh = P(now, vec![]);
        assert!(
            plan(&dirs, u64::MAX, ORPHAN_IDLE, &fresh).is_empty(),
            "recent"
        );
        let later = P(
            now + ORPHAN_IDLE.as_secs() + 10,
            vec![base.join("busy/src"), base.join("elsewhere")],
        );
        let p = plan(&dirs, u64::MAX, ORPHAN_IDLE, &later);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(
            p.iter().any(|o| o.dir == loose),
            "shared parent in use is fine"
        );
        let p: Vec<_> = p.into_iter().filter(|o| o.dir == old).collect();
        assert_eq!(plan(&dirs, 0, ORPHAN_IDLE, &later), vec![]);
        // A build touching it between plan and delete wins.
        assert!(evict(&p[0], ORPHAN_IDLE, &fresh).is_err());
        // The refused attempt released the parent's lock, but a child another
        // test is forking can hold its copy until it execs.
        crate::test_fork::relock(|| evict(&p[0], ORPHAN_IDLE, &later)).unwrap();
        assert!(!old.exists());
        assert!(base.join("old/src.rs").exists());
        assert!(busy.exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn sent_sweep_drops_old_pairs_only() {
        let base = super::super::tests::scratch("orphan-sent");
        for f in ["old", "old.id", "new", "new.id", "x.new"] {
            std::fs::write(base.join(f), "data").unwrap();
        }
        let set = |f: &str, age: u64| {
            let t = SystemTime::now() - Duration::from_secs(age);
            std::fs::File::options()
                .write(true)
                .open(base.join(f))
                .unwrap()
                .set_modified(t)
                .unwrap();
        };
        set("old", 10 * 86400);
        set("x.new", 7200);
        let (n, _) = sweep_sent(&base, SENT_MAX_AGE, SystemTime::now());
        assert_eq!(n, 3);
        assert!(!base.join("old").exists() && !base.join("old.id").exists());
        assert!(base.join("new").exists() && base.join("new.id").exists());
        assert!(!base.join("x.new").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn bases_cover_mirror_parents_and_caches() {
        let h = Path::new("/home/u");
        let b = bases(
            h,
            &[PathBuf::from("/home/j/proj"), PathBuf::from("/home/j/x/y")],
        );
        assert_eq!(
            b,
            vec![
                PathBuf::from("/home/j"),
                PathBuf::from("/home/u/.cache"),
                PathBuf::from("/home/u/.jcode/scratch")
            ]
        );
    }

    #[test]
    fn runs_sweep_keeps_newest_and_prunes_old() {
        let base = super::super::tests::scratch("orphan-runs");
        let now = SystemTime::now();
        for (name, age_days) in [
            ("r-old-1", 10),
            ("r-old-2", 9),
            ("r-keep-old", 8),
            ("r-new", 1),
        ] {
            let d = base.join(name);
            std::fs::create_dir(&d).unwrap();
            let t = now - Duration::from_secs(age_days * 86400);
            std::fs::File::open(&d).unwrap().set_modified(t).unwrap();
        }
        // Keep newest 2 unconditionally, prune older than 7 days beyond that.
        let removed = sweep_runs(&base, Duration::from_secs(7 * 86400), 2, now);
        assert_eq!(removed, 2);
        assert!(!base.join("r-old-1").exists());
        assert!(!base.join("r-old-2").exists());
        assert!(base.join("r-keep-old").exists());
        assert!(base.join("r-new").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn tmp_sweep_removes_only_old_unused_build_temps() {
        let base = super::super::tests::scratch("orphan-tmp");
        let now = SystemTime::now();
        let old = now - Duration::from_secs(48 * 3600);
        for name in ["jr-old", "rustc-busy", "justrust-fresh", "user-notes"] {
            let d = base.join(name);
            std::fs::create_dir(&d).unwrap();
            if name != "justrust-fresh" {
                std::fs::File::open(&d).unwrap().set_modified(old).unwrap();
            }
        }
        let probe = P(secs(now), vec![base.join("rustc-busy/tmp")]);
        let removed = sweep_tmp(&base, TMP_MAX_AGE, now, &probe);
        assert_eq!(removed, 1);
        assert!(!base.join("jr-old").exists());
        assert!(base.join("rustc-busy").exists());
        assert!(base.join("justrust-fresh").exists());
        assert!(base.join("user-notes").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn toolchain_sweep_spares_slots_policy_and_recent_jr() {
        let home = super::super::tests::scratch("orphan-tc");
        let tc = home.join("toolchains");
        let staged = tc.join("staged");
        std::fs::create_dir_all(&staged).unwrap();
        let now = SystemTime::now();
        let old = now - Duration::from_secs(10 * 86400);
        for dir in [
            tc.join("jr-1111111"),
            tc.join("jr-2222222"),
            tc.join("jr-3333333"),
            staged.join("jr-4444444"),
            tc.join("nightly-2026-10-08"),
        ] {
            std::fs::create_dir(&dir).unwrap();
            std::fs::File::open(&dir)
                .unwrap()
                .set_modified(old)
                .unwrap();
        }
        std::fs::write(
            tc.join(".slots.json"),
            r#"{"version":1,"active":{"id":"jr-1111111","path":"/x"},"last_good":"jr-1111111","previous":null}"#,
        )
        .unwrap();
        std::fs::write(
            home.join("toolchain-policy.json"),
            r#"{"toolchain":"jr-2222222","protocol":1,"fetched_at":0}"#,
        )
        .unwrap();
        let probe = P(secs(now), vec![]);
        let removed = sweep_toolchains(&home, TOOLCHAIN_MAX_AGE, now, &probe);
        assert_eq!(removed, 2);
        assert!(tc.join("jr-1111111").exists(), "active slot kept");
        assert!(tc.join("jr-2222222").exists(), "policy toolchain kept");
        assert!(tc.join("nightly-2026-10-08").exists(), "non-jr kept");
        assert!(!tc.join("jr-3333333").exists(), "stale jr pruned");
        assert!(!staged.join("jr-4444444").exists(), "stale staged pruned");
        std::fs::remove_dir_all(&home).unwrap();
    }
}
