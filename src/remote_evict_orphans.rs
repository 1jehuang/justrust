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
        evict(&p[0], ORPHAN_IDLE, &later).unwrap();
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
}
