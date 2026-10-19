//! Hosted build machines: evict mirrored `target/` dirs under low disk.
//!
//! Every source root a client mirrors onto the machine keeps a full `target/`
//! (10-17 GB for large crates). Nothing removed them, and on 2026-10-19 they
//! filled a 484 GB hosted disk, failing every hosted build. This collector
//! frees space by deleting only rebuildable build output:
//!
//! - Scope: only `<root>/target` where `<root>` is a source root this agent
//!   was told to mirror (the registry), never sources, `.git`, `.justrust`,
//!   cargo homes, shared caches, or a `CARGO_TARGET_DIR` outside a root.
//! - Path authority: `<root>/target` must be a real directory (no symlink in
//!   it or any ancestor up to the root), on the root's filesystem, and its
//!   canonical path must be exactly `canonical(root)/target`.
//! - Never while in use: a root with a running build (lease), or built within
//!   the grace period, or used by any other process (cwd or open fd under it),
//!   is skipped. Builds and eviction exclude each other per root with a flock.
//! - Trigger: when free space is below the low watermark, evict least
//!   recently built targets until above the high watermark.
//! - `JUSTRUST_REMOTE_EVICT`: `dry` (default) only logs what it would delete,
//!   `on` deletes, `0`/`off` disables.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Keep free space at least this fraction of the disk, or this many bytes,
/// whichever is larger.
pub const LOW_FRACTION: f64 = 0.15;
pub const LOW_BYTES: u64 = 50 << 30;
/// Once triggered, evict until this fraction is free.
pub const HIGH_FRACTION: f64 = 0.25;
/// A root built more recently than this is never evicted.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(2 * 3600);
/// Below this, refuse to start a build (after an eviction attempt).
pub const FLOOR_BYTES: u64 = 5 << 30;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Off,
    Dry,
    On,
}

impl Mode {
    pub fn from_env() -> Mode {
        match std::env::var("JUSTRUST_REMOTE_EVICT").as_deref() {
            Ok("0" | "off" | "false") => Mode::Off,
            Ok("1" | "on" | "true") => Mode::On,
            _ => Mode::Dry,
        }
    }
}

/// What the collector knows about one mirrored root.
#[derive(Clone, Debug, Default)]
pub struct RootState {
    /// Builds currently running with this root as their cwd.
    pub leases: u32,
    /// Unix seconds when the last build in this root ended (0: never).
    pub last_build_end: u64,
}

/// Environment the collector observes. Injected so tests are hermetic.
pub trait Probe {
    /// (free bytes, total bytes) of the filesystem holding `path`.
    fn disk(&self, path: &Path) -> Option<(u64, u64)>;
    /// Bytes used under `path` (not following symlinks).
    fn size(&self, path: &Path) -> u64;
    /// Some process other than this agent has its cwd or an open file under
    /// `path`.
    fn in_use_by_other(&self, path: &Path) -> bool;
    fn now(&self) -> u64;
}

/// One deletion decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Eviction {
    pub target: PathBuf,
    pub bytes: u64,
    pub last_build_end: u64,
}

/// Why a candidate was skipped, for the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Skip {
    Missing,
    Unsafe(String),
    Leased,
    Recent,
    InUse,
}

/// `<root>/target` if it is safe to delete, else why not.
pub fn target_of(root: &Path) -> Result<PathBuf, Skip> {
    let root_meta = std::fs::symlink_metadata(root).map_err(|_| Skip::Missing)?;
    if root_meta.file_type().is_symlink() || !root_meta.is_dir() {
        return Err(Skip::Unsafe("root is not a real directory".into()));
    }
    let target = root.join("target");
    let meta = match std::fs::symlink_metadata(&target) {
        Ok(m) => m,
        Err(_) => return Err(Skip::Missing),
    };
    if meta.file_type().is_symlink() {
        return Err(Skip::Unsafe("target is a symlink".into()));
    }
    if !meta.is_dir() {
        return Err(Skip::Unsafe("target is not a directory".into()));
    }
    let canonical_root = root
        .canonicalize()
        .map_err(|e| Skip::Unsafe(format!("root: {e}")))?;
    if canonical_root != root {
        // An ancestor of the registered root is a symlink: the registered
        // path does not name the directory it appears to.
        return Err(Skip::Unsafe("root path goes through a symlink".into()));
    }
    let canonical = target
        .canonicalize()
        .map_err(|e| Skip::Unsafe(format!("target: {e}")))?;
    if canonical != canonical_root.join("target") {
        return Err(Skip::Unsafe("target resolves outside the root".into()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.dev() != root_meta.dev() {
            return Err(Skip::Unsafe("target is on another filesystem".into()));
        }
    }
    Ok(target)
}

/// Bytes that must be freed now, or 0 if free space is above the low
/// watermark.
pub fn deficit(free: u64, total: u64) -> u64 {
    let low = ((total as f64 * LOW_FRACTION) as u64).max(LOW_BYTES.min(total / 2));
    if free >= low {
        return 0;
    }
    let high = ((total as f64 * HIGH_FRACTION) as u64).max(low);
    high.saturating_sub(free)
}

/// Decide what to evict: eligible targets, least recently built first, until
/// `need` bytes are covered. Returns the plan and the skipped roots.
pub fn plan(
    roots: &BTreeMap<PathBuf, RootState>,
    need: u64,
    grace: Duration,
    probe: &dyn Probe,
) -> (Vec<Eviction>, Vec<(PathBuf, Skip)>) {
    let now = probe.now();
    let mut candidates = Vec::new();
    let mut skipped = Vec::new();
    for (root, state) in roots {
        if state.leases > 0 {
            skipped.push((root.clone(), Skip::Leased));
            continue;
        }
        if state.last_build_end.saturating_add(grace.as_secs()) > now {
            skipped.push((root.clone(), Skip::Recent));
            continue;
        }
        let target = match target_of(root) {
            Ok(t) => t,
            Err(why) => {
                skipped.push((root.clone(), why));
                continue;
            }
        };
        if probe.in_use_by_other(root) {
            skipped.push((root.clone(), Skip::InUse));
            continue;
        }
        let bytes = probe.size(&target);
        candidates.push(Eviction {
            target,
            bytes,
            last_build_end: state.last_build_end,
        });
    }
    candidates.sort_by_key(|c| (c.last_build_end, std::cmp::Reverse(c.bytes)));
    let mut chosen = Vec::new();
    let mut freed = 0u64;
    for c in candidates {
        if freed >= need {
            break;
        }
        freed = freed.saturating_add(c.bytes);
        chosen.push(c);
    }
    (chosen, skipped)
}

/// Registry of mirrored roots and their build activity, owned by the agent.
#[derive(Default)]
pub struct Registry {
    pub roots: BTreeMap<PathBuf, RootState>,
}

impl Registry {
    /// A root became known through a sync.
    pub fn touch(&mut self, root: &Path) {
        self.roots.entry(root.to_path_buf()).or_default();
    }
    /// The registered root containing `cwd`, if any.
    pub fn root_of(&self, cwd: &Path) -> Option<PathBuf> {
        self.roots
            .keys()
            .filter(|r| cwd.starts_with(r))
            .max_by_key(|r| r.as_os_str().len())
            .cloned()
    }
    pub fn begin(&mut self, root: &Path) {
        self.roots.entry(root.to_path_buf()).or_default().leases += 1;
    }
    pub fn end(&mut self, root: &Path, now: u64) {
        let s = self.roots.entry(root.to_path_buf()).or_default();
        s.leases = s.leases.saturating_sub(1);
        s.last_build_end = now;
    }
}

/// Per-root lock shared by builds and exclusive for eviction, so a build
/// never starts on a half-deleted `target/`.
pub struct RootLock(#[allow(dead_code)] std::fs::File);

fn lock_path(root: &Path) -> Option<PathBuf> {
    let name = format!("{:016x}.lock", fxhash(root.as_os_str().as_encoded_bytes()));
    let dir = crate::remote::dir().ok()?.join("evict-locks");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(name))
}

fn fxhash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[cfg(unix)]
fn flock(root: &Path, exclusive: bool, blocking: bool) -> Option<RootLock> {
    use std::os::fd::AsRawFd;
    let path = lock_path(root)?;
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .ok()?;
    let mut op = if exclusive {
        libc::LOCK_EX
    } else {
        libc::LOCK_SH
    };
    if !blocking {
        op |= libc::LOCK_NB;
    }
    // SAFETY: flock on an owned, open descriptor; released when `f` drops.
    (unsafe { libc::flock(f.as_raw_fd(), op) } == 0).then_some(RootLock(f))
}

/// Held by a build for its whole duration (blocks while eviction runs).
#[cfg(unix)]
pub fn build_lock(root: &Path) -> Option<RootLock> {
    flock(root, false, true)
}

/// Taken by eviction; fails immediately if any build holds the root.
#[cfg(unix)]
pub fn evict_lock(root: &Path) -> Option<RootLock> {
    flock(root, true, false)
}

/// Delete a planned target, re-checking path authority under the lock.
pub fn evict(e: &Eviction) -> std::io::Result<()> {
    let Some(root) = e.target.parent() else {
        return Err(std::io::Error::other("target has no root"));
    };
    match target_of(root) {
        Ok(t) if t == e.target => std::fs::remove_dir_all(&e.target),
        Ok(_) => Err(std::io::Error::other("target changed")),
        Err(why) => Err(std::io::Error::other(format!("{why:?}"))),
    }
}

/// The real machine.
pub struct System;

impl Probe for System {
    fn disk(&self, path: &Path) -> Option<(u64, u64)> {
        #[cfg(unix)]
        {
            let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
            // SAFETY: statvfs fills the zeroed struct; the path is NUL-terminated.
            let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
            if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
                return None;
            }
            let frsize = st.f_frsize as u64;
            Some((
                (st.f_bavail as u64).saturating_mul(frsize),
                (st.f_blocks as u64).saturating_mul(frsize),
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = path;
            None
        }
    }
    fn size(&self, path: &Path) -> u64 {
        fn walk(p: &Path) -> u64 {
            let Ok(meta) = std::fs::symlink_metadata(p) else {
                return 0;
            };
            if meta.is_dir() {
                std::fs::read_dir(p)
                    .map(|rd| rd.flatten().map(|e| walk(&e.path())).sum())
                    .unwrap_or(0)
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::MetadataExt;
                    meta.blocks().saturating_mul(512)
                }
                #[cfg(not(unix))]
                meta.len()
            }
        }
        walk(path)
    }
    fn in_use_by_other(&self, path: &Path) -> bool {
        let me = std::process::id().to_string();
        let Ok(procs) = std::fs::read_dir("/proc") else {
            return true;
        };
        for p in procs.flatten() {
            let name = p.file_name();
            let Some(pid) = name
                .to_str()
                .filter(|s| s.bytes().all(|b| b.is_ascii_digit()))
            else {
                continue;
            };
            if pid == me {
                continue;
            }
            let base = p.path();
            if std::fs::read_link(base.join("cwd")).is_ok_and(|c| c.starts_with(path)) {
                return true;
            }
            if let Ok(fds) = std::fs::read_dir(base.join("fd")) {
                for fd in fds.flatten() {
                    if std::fs::read_link(fd.path()).is_ok_and(|t| t.starts_with(path)) {
                        return true;
                    }
                }
            }
        }
        false
    }
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// One collection pass: plan, lock, delete (unless dry), log every decision.
/// Returns bytes freed (or that would be freed in dry mode).
#[cfg(unix)]
pub fn collect(
    registry: &Registry,
    home: &Path,
    mode: Mode,
    grace: Duration,
    probe: &dyn Probe,
    log: &mut dyn FnMut(String),
) -> u64 {
    if mode == Mode::Off {
        return 0;
    }
    let Some((free, total)) = probe.disk(home) else {
        return 0;
    };
    let need = deficit(free, total);
    if need == 0 {
        return 0;
    }
    let (chosen, skipped) = plan(&registry.roots, need, grace, probe);
    for (root, why) in &skipped {
        log(format!("evict: skip {} ({why:?})", root.display()));
    }
    let mut freed = 0u64;
    for e in chosen {
        let gib = e.bytes as f64 / (1u64 << 30) as f64;
        if mode == Mode::Dry {
            log(format!(
                "evict (dry run): would delete {} ({gib:.1} GiB, last build {})",
                e.target.display(),
                e.last_build_end
            ));
            freed += e.bytes;
            continue;
        }
        let Some(root) = e.target.parent() else {
            continue;
        };
        let Some(_lock) = evict_lock(root) else {
            log(format!("evict: skip {} (build holds lock)", root.display()));
            continue;
        };
        match evict(&e) {
            Ok(()) => {
                log(format!(
                    "evict: deleted {} ({gib:.1} GiB, last build {}, free was {free} of {total})",
                    e.target.display(),
                    e.last_build_end
                ));
                freed += e.bytes;
            }
            Err(err) => log(format!("evict: failed {}: {err}", e.target.display())),
        }
    }
    freed
}

/// Free space after an attempt, for the pre-build floor check.
pub fn below_floor(probe: &dyn Probe, home: &Path) -> Option<u64> {
    let (free, _) = probe.disk(home)?;
    (free < FLOOR_BYTES).then_some(free)
}

/// Builds in progress per root, shared between the serve loop and run threads.
pub type Shared = std::sync::Arc<std::sync::Mutex<Registry>>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Fake {
        free: u64,
        total: u64,
        sizes: HashMap<PathBuf, u64>,
        busy: Vec<PathBuf>,
        now: u64,
    }

    impl Probe for Fake {
        fn disk(&self, _: &Path) -> Option<(u64, u64)> {
            Some((self.free, self.total))
        }
        fn size(&self, p: &Path) -> u64 {
            self.sizes.get(p).copied().unwrap_or(0)
        }
        fn in_use_by_other(&self, p: &Path) -> bool {
            self.busy.iter().any(|b| b.starts_with(p))
        }
        fn now(&self) -> u64 {
            self.now
        }
    }

    fn scratch(tag: &str) -> PathBuf {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let d = base.join(format!(
            "jr-evict-{tag}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&d).unwrap();
        d.canonicalize().unwrap()
    }

    fn mirrored(base: &Path, name: &str) -> PathBuf {
        let root = base.join(name);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/lib.rs"), "fn f() {}").unwrap();
        std::fs::create_dir_all(root.join("target/debug")).unwrap();
        std::fs::write(root.join("target/debug/big"), vec![0u8; 4096]).unwrap();
        root
    }

    fn fake(free: u64, total: u64, now: u64) -> Fake {
        Fake {
            free,
            total,
            sizes: HashMap::new(),
            busy: Vec::new(),
            now,
        }
    }

    const GIB: u64 = 1 << 30;

    // T1 allowlist: only registered roots' target/ is ever a candidate.
    #[test]
    fn only_registered_root_targets_are_candidates() {
        let base = scratch("allow");
        let a = mirrored(&base, "a");
        let b = mirrored(&base, "b");
        let outside = base.join("custom-target");
        std::fs::create_dir_all(&outside).unwrap();
        let mut reg = Registry::default();
        reg.touch(&a);
        let mut p = fake(GIB, 484 * GIB, 100_000);
        p.sizes.insert(a.join("target"), 10 * GIB);
        p.sizes.insert(b.join("target"), 10 * GIB);
        let (chosen, _) = plan(&reg.roots, u64::MAX, DEFAULT_GRACE, &p);
        assert_eq!(
            chosen.iter().map(|e| e.target.clone()).collect::<Vec<_>>(),
            vec![a.join("target")]
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    // T2 symlink defense: a symlinked target or root is never deleted, and
    // deleting a real target never follows symlinks inside it.
    #[cfg(unix)]
    #[test]
    fn symlinks_are_refused_and_never_followed() {
        let base = scratch("symlink");
        let victim = base.join("victim");
        std::fs::create_dir_all(&victim).unwrap();
        std::fs::write(victim.join("keep"), "precious").unwrap();

        let a = base.join("a");
        std::fs::create_dir_all(a.join("src")).unwrap();
        std::os::unix::fs::symlink(&victim, a.join("target")).unwrap();
        assert!(matches!(target_of(&a), Err(Skip::Unsafe(_))));

        let real = mirrored(&base, "real");
        let linked_root = base.join("linked-root");
        std::os::unix::fs::symlink(&real, &linked_root).unwrap();
        assert!(matches!(target_of(&linked_root), Err(Skip::Unsafe(_))));

        let c = mirrored(&base, "c");
        std::os::unix::fs::symlink(&victim, c.join("target/escape")).unwrap();
        let e = Eviction {
            target: target_of(&c).unwrap(),
            bytes: 1,
            last_build_end: 0,
        };
        evict(&e).unwrap();
        assert!(!c.join("target").exists());
        assert!(c.join("src/lib.rs").exists(), "sources kept");
        assert_eq!(
            std::fs::read_to_string(victim.join("keep")).unwrap(),
            "precious"
        );
        std::fs::remove_dir_all(&base).unwrap();
    }

    // T3 leases and grace.
    #[test]
    fn running_and_recent_builds_are_not_evicted() {
        let base = scratch("lease");
        let a = mirrored(&base, "a");
        let mut reg = Registry::default();
        reg.touch(&a);
        reg.begin(&a);
        let p = fake(GIB, 484 * GIB, 1_000_000);
        let (chosen, skipped) = plan(&reg.roots, u64::MAX, DEFAULT_GRACE, &p);
        assert!(chosen.is_empty());
        assert_eq!(skipped[0].1, Skip::Leased);
        reg.end(&a, 1_000_000 - 60);
        let (chosen, skipped) = plan(&reg.roots, u64::MAX, DEFAULT_GRACE, &p);
        assert!(chosen.is_empty());
        assert_eq!(skipped[0].1, Skip::Recent);
        let later = fake(GIB, 484 * GIB, 1_000_000 + DEFAULT_GRACE.as_secs());
        let (chosen, _) = plan(&reg.roots, u64::MAX, DEFAULT_GRACE, &later);
        assert_eq!(chosen.len(), 1);
        std::fs::remove_dir_all(&base).unwrap();
    }

    // T3b any other process using the root blocks eviction.
    #[test]
    fn roots_in_use_by_other_processes_are_skipped() {
        let base = scratch("busy");
        let a = mirrored(&base, "a");
        let mut reg = Registry::default();
        reg.touch(&a);
        let mut p = fake(GIB, 484 * GIB, 1_000_000);
        p.busy.push(a.join("target/debug"));
        let (chosen, skipped) = plan(&reg.roots, u64::MAX, DEFAULT_GRACE, &p);
        assert!(chosen.is_empty());
        assert_eq!(skipped[0].1, Skip::InUse);
        std::fs::remove_dir_all(&base).unwrap();
    }

    // T4 a build's shared lock blocks eviction of that root.
    #[cfg(unix)]
    #[test]
    fn build_lock_blocks_eviction_lock() {
        let base = scratch("lock");
        let a = mirrored(&base, "a");
        let held = build_lock(&a).expect("build lock");
        assert!(
            evict_lock(&a).is_none(),
            "eviction must not proceed during a build"
        );
        drop(held);
        let ev = evict_lock(&a).expect("free after build");
        drop(ev);
        std::fs::remove_dir_all(&base).unwrap();
    }

    // T5 watermark and LRU order, stopping once enough is freed.
    #[test]
    fn evicts_oldest_until_above_high_watermark() {
        let base = scratch("lru");
        let mut reg = Registry::default();
        let mut p = fake(10 * GIB, 400 * GIB, 10_000_000);
        for (name, end) in [("old", 1_000), ("mid", 2_000), ("new", 3_000)] {
            let r = mirrored(&base, name);
            reg.touch(&r);
            reg.roots.get_mut(&r).unwrap().last_build_end = end;
            p.sizes.insert(r.join("target"), 50 * GIB);
        }
        let need = deficit(p.free, p.total);
        assert_eq!(need, 90 * GIB, "25% of 400 GiB minus 10 GiB free");
        let (chosen, _) = plan(&reg.roots, need, DEFAULT_GRACE, &p);
        let names: Vec<_> = chosen
            .iter()
            .map(|e| e.target.parent().unwrap().file_name().unwrap().to_owned())
            .collect();
        assert_eq!(names, ["old", "mid"]);
        assert_eq!(deficit(200 * GIB, 400 * GIB), 0, "plenty free: nothing");
        std::fs::remove_dir_all(&base).unwrap();
    }

    // T6 dry run deletes nothing but reports exact paths and sizes.
    #[cfg(unix)]
    #[test]
    fn dry_run_deletes_nothing() {
        let base = scratch("dry");
        let a = mirrored(&base, "a");
        let mut reg = Registry::default();
        reg.touch(&a);
        let mut p = fake(GIB, 484 * GIB, 10_000_000);
        p.sizes.insert(a.join("target"), 12 * GIB);
        let mut lines = Vec::new();
        let freed = collect(&reg, &base, Mode::Dry, DEFAULT_GRACE, &p, &mut |l| {
            lines.push(l)
        });
        assert_eq!(freed, 12 * GIB);
        assert!(a.join("target/debug/big").exists());
        assert!(lines.iter().any(|l| l.contains("would delete")
            && l.contains(&a.join("target").display().to_string())
            && l.contains("12.0 GiB")));
        assert_eq!(
            collect(&reg, &base, Mode::Off, DEFAULT_GRACE, &p, &mut |_| {}),
            0
        );
        let freed = collect(&reg, &base, Mode::On, DEFAULT_GRACE, &p, &mut |_| {});
        assert_eq!(freed, 12 * GIB);
        assert!(!a.join("target").exists());
        assert!(a.join("src/lib.rs").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    // T9 a seeded sibling root keeps its own target.
    #[test]
    fn evicting_one_root_leaves_siblings_intact() {
        let base = scratch("sibling");
        let a = mirrored(&base, "a");
        let b = mirrored(&base, "b");
        let e = Eviction {
            target: target_of(&a).unwrap(),
            bytes: 1,
            last_build_end: 0,
        };
        evict(&e).unwrap();
        assert!(!a.join("target").exists());
        assert!(b.join("target/debug/big").exists());
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn mode_parsing_defaults_to_dry() {
        if std::env::var_os("JUSTRUST_REMOTE_EVICT").is_none() {
            assert_eq!(Mode::from_env(), Mode::Dry);
        }
    }

    #[test]
    fn registry_maps_cwd_to_the_innermost_root() {
        let mut reg = Registry::default();
        reg.touch(Path::new("/w/a"));
        reg.touch(Path::new("/w/a/nested"));
        assert_eq!(
            reg.root_of(Path::new("/w/a/nested/crate")),
            Some(PathBuf::from("/w/a/nested"))
        );
        assert_eq!(reg.root_of(Path::new("/w/other")), None);
    }
}
