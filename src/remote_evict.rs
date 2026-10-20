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
//! - `JUSTRUST_REMOTE_EVICT`: `on` (default) deletes, `dry` only logs what it
//!   would delete, `0`/`off` disables.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

#[cfg(unix)]
#[path = "remote_evict_registry.rs"]
mod durable;

#[cfg(test)]
#[path = "remote_evict_lock_tests.rs"]
mod lock_tests;

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
            Ok("dry") => Mode::Dry,
            _ => Mode::On,
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
        if meta.uid() != unsafe { libc::geteuid() } || root_meta.uid() != meta.uid() {
            return Err(Skip::Unsafe(
                "root/target belongs to another principal".into(),
            ));
        }
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

type Snapshot = (BTreeMap<PathBuf, RootState>, Vec<(PathBuf, Skip)>);

/// Registry of mirrored roots and their build activity, owned by the agent.
#[derive(Default)]
pub struct Registry {
    pub roots: BTreeMap<PathBuf, RootState>,
    #[cfg(unix)]
    store: Option<durable::Store>,
    disabled: Option<String>,
}

impl Registry {
    /// Load only this SSH principal's authenticated enrollment receipts.
    #[cfg(unix)]
    pub fn open(prefix: &str) -> std::io::Result<Self> {
        Self::from_store(durable::Store::open(prefix)?)
    }

    #[cfg(unix)]
    fn from_store(store: durable::Store) -> std::io::Result<Self> {
        let (mut roots, _) = store.snapshot()?;
        for state in roots.values_mut() {
            state.leases = 0;
        }
        Ok(Self {
            roots,
            store: Some(store),
            disabled: None,
        })
    }

    pub fn disabled(reason: String) -> Self {
        Self {
            disabled: Some(reason),
            ..Self::default()
        }
    }

    pub fn register(&mut self, root: &Path) -> std::io::Result<()> {
        // Remember synchronization roots even if durable enrollment fails, so
        // a subsequent run cannot bypass the cross-agent lock/record checks.
        self.roots.entry(root.to_path_buf()).or_default();
        #[cfg(unix)]
        if let Some(store) = &self.store {
            let _lock = build_lock(root)?;
            store.register(root)?;
            self.roots
                .entry(root.to_path_buf())
                .or_default()
                .last_build_end = store.state(root)?.last_build_end;
            return Ok(());
        }
        self.touch(root);
        Ok(())
    }

    /// Fresh receipts prevent another agent's activity being lost on upgrade.
    #[cfg(unix)]
    fn snapshot(&self) -> std::io::Result<Snapshot> {
        if let Some(reason) = &self.disabled {
            return Err(std::io::Error::other(reason.clone()));
        }
        let Some(store) = &self.store else {
            return Ok((self.roots.clone(), Vec::new()));
        };
        let (mut roots, skipped) = store.snapshot()?;
        for (root, state) in &mut roots {
            if let Some(local) = self.roots.get(root) {
                state.leases = state.leases.max(local.leases);
                state.last_build_end = state.last_build_end.max(local.last_build_end);
            }
        }
        Ok((roots, skipped))
    }

    #[cfg(unix)]
    fn fresh_state(&self, root: &Path) -> std::io::Result<RootState> {
        let mut state = match &self.store {
            Some(store) => store.state(root)?,
            None => self
                .roots
                .get(root)
                .cloned()
                .ok_or_else(|| std::io::Error::other("unregistered root"))?,
        };
        if let Some(local) = self.roots.get(root) {
            state.leases = state.leases.max(local.leases);
            state.last_build_end = state.last_build_end.max(local.last_build_end);
        }
        Ok(state)
    }

    fn record_activity(&self, root: &Path) -> std::io::Result<()> {
        #[cfg(unix)]
        if let Some(store) = &self.store {
            store.activity(root)?;
        }
        Ok(())
    }

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
    pub fn begin(&mut self, root: &Path) -> std::io::Result<()> {
        if let Some(reason) = &self.disabled {
            return Err(std::io::Error::other(reason.clone()));
        }
        #[cfg(unix)]
        if let Some(store) = &self.store {
            store.begin(root)?;
        }
        self.roots.entry(root.to_path_buf()).or_default().leases += 1;
        Ok(())
    }
    pub fn end(&mut self, root: &Path, now: u64) -> std::io::Result<()> {
        self.record_activity(root)?;
        let s = self.roots.entry(root.to_path_buf()).or_default();
        s.leases = s.leases.saturating_sub(1);
        s.last_build_end = now;
        Ok(())
    }
    /// Clear the durable uncertainty marker only after every agent has
    /// released its shared build lock. A crashed/failed activity write stays
    /// protected until a later successful build finishes cleanly.
    #[cfg(unix)]
    pub fn finish_idle(&self, root: &Path) -> std::io::Result<()> {
        if self.roots.get(root).is_some_and(|s| s.leases != 0) {
            return Ok(());
        }
        let _lock = match evict_lock(root) {
            Ok(lock) => lock,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => return Ok(()),
            Err(e) => return Err(e),
        };
        if let Some(store) = &self.store {
            store.finish(root)?;
        }
        Ok(())
    }
}

/// Per-root lock shared by builds and exclusive for eviction, so a build
/// never starts on a half-deleted `target/`.
pub struct RootLock(#[allow(dead_code)] std::fs::File);

fn validate_lock_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o022 != 0
        || dir.canonicalize()? != dir
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            format!("unsafe source-root lock directory {}", dir.display()),
        ));
    }
    Ok(())
}

fn lock_path(root: &Path) -> std::io::Result<PathBuf> {
    let name = format!("{:016x}.lock", fxhash(root.as_os_str().as_encoded_bytes()));
    let dir = crate::remote::dir()
        .map_err(std::io::Error::other)?
        .join("evict-locks");
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&dir)?;
    validate_lock_dir(&dir)?;
    Ok(dir.join(name))
}

fn fxhash(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn validate_lock_inode(meta: &std::fs::Metadata, uid: u32) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if !meta.is_file() || meta.uid() != uid || meta.nlink() != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "lock is not an owned regular single-link inode",
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn flock_at(path: &Path, exclusive: bool, blocking: bool) -> std::io::Result<RootLock> {
    use std::os::fd::AsRawFd;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let acquire = || {
        validate_lock_dir(
            path.parent()
                .ok_or_else(|| std::io::Error::other("lock has no parent"))?,
        )?;
        let f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
            .open(path)?;
        let meta = f.metadata()?;
        validate_lock_inode(&meta, unsafe { libc::geteuid() })?;
        // Old agents created these with mode 0666 & umask (usually 0664).
        // Tighten the SAME verified descriptor. Never unlink/recreate or drop
        // another process's flock: existing shared holders remain authoritative.
        if meta.mode() & 0o7777 != 0o600 {
            f.set_permissions(std::fs::Permissions::from_mode(0o600))?;
            f.sync_all()?;
        }
        let mut op = if exclusive {
            libc::LOCK_EX
        } else {
            libc::LOCK_SH
        };
        if !blocking {
            op |= libc::LOCK_NB;
        }
        loop {
            // SAFETY: owned open descriptor, flock released only on its drop.
            if unsafe { libc::flock(f.as_raw_fd(), op) } == 0 {
                return Ok(RootLock(f));
            }
            let error = std::io::Error::last_os_error();
            if blocking && error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(error);
        }
    };
    acquire().map_err(|e| {
        std::io::Error::new(
            e.kind(),
            format!("source-root lock {}: {e}", path.display()),
        )
    })
}

/// Held by a build for its whole duration (blocks while eviction runs).
#[cfg(unix)]
pub fn build_lock(root: &Path) -> std::io::Result<RootLock> {
    flock_at(&lock_path(root)?, false, true)
}

/// Taken by eviction; WouldBlock means an existing build still holds this inode.
#[cfg(unix)]
pub fn evict_lock(root: &Path) -> std::io::Result<RootLock> {
    flock_at(&lock_path(root)?, true, false)
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
        crate::procfs::path_in_use_by_other(path)
    }
    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs())
    }
}

/// One collection pass. Return allocated tree bytes, not physical bytes
/// reclaimed: hardlinks and concurrent filesystem activity can differ.
#[cfg(unix)]
pub fn collect(
    registry: &Registry,
    home: &Path,
    mode: Mode,
    grace: Duration,
    probe: &dyn Probe,
    log: &mut dyn FnMut(String),
) -> u64 {
    collect_report(registry, home, mode, grace, probe, log).bytes
}

#[derive(Default)]
struct Collection {
    bytes: u64,
    failed: bool,
}

#[cfg(unix)]
fn collect_report(
    registry: &Registry,
    home: &Path,
    mode: Mode,
    grace: Duration,
    probe: &dyn Probe,
    log: &mut dyn FnMut(String),
) -> Collection {
    if mode == Mode::Off {
        return Collection::default();
    }
    let Some((free, total)) = probe.disk(home) else {
        log("evict: cannot measure filesystem, refusing collection".into());
        return Collection {
            failed: true,
            ..Collection::default()
        };
    };
    let need = deficit(free, total);
    if need == 0 {
        return Collection::default();
    }
    let (roots, mut rejected) = match registry.snapshot() {
        Ok(snapshot) => snapshot,
        Err(e) => {
            log(format!(
                "evict: registry unavailable, refusing collection: {e}"
            ));
            return Collection {
                failed: true,
                ..Collection::default()
            };
        }
    };
    log(format!(
        "evict: registered={} free_bytes={free} total_bytes={total} needed_bytes={need} grace_seconds={}",
        roots.len(),
        grace.as_secs()
    ));
    let (chosen, skipped) = plan(&roots, need, grace, probe);
    rejected.extend(skipped);
    for (root, why) in &rejected {
        log(format!("evict: skip {} ({why:?})", root.display()));
    }
    let mut freed = 0u64;
    let mut failed = false;
    for e in chosen {
        let gib = e.bytes as f64 / (1u64 << 30) as f64;
        if mode == Mode::Dry {
            log(format!(
                "evict (dry run): would delete {} ({gib:.1} GiB, allocated_tree_bytes={}, last activity {})",
                e.target.display(),
                e.bytes,
                e.last_build_end
            ));
            freed += e.bytes;
            continue;
        }
        let Some(root) = e.target.parent() else {
            continue;
        };
        let _lock = match evict_lock(root) {
            Ok(lock) => lock,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                log(format!("evict: skip {} (build holds lock)", root.display()));
                continue;
            }
            Err(e) => {
                failed = true;
                log(format!("evict: failed root lock {}: {e}", root.display()));
                continue;
            }
        };
        // A different agent can finish a build between planning and flock.
        // Reload authenticated durable activity while holding exclusive access.
        let state = match registry.fresh_state(root) {
            Ok(s) => s,
            Err(err) => {
                log(format!(
                    "evict: skip {} (fresh receipt unavailable: {err})",
                    root.display()
                ));
                continue;
            }
        };
        if state.leases != 0
            || state.last_build_end.saturating_add(grace.as_secs()) > probe.now()
            || probe.in_use_by_other(root)
        {
            log(format!(
                "evict: skip {} (fresh lease/recent/process check)",
                root.display()
            ));
            continue;
        }
        match evict(&e) {
            Ok(()) => {
                log(format!(
                    "evict: deleted {} ({gib:.1} GiB, allocated_tree_bytes={}, last activity {}, free was {free} of {total})",
                    e.target.display(),
                    e.bytes,
                    state.last_build_end
                ));
                freed += e.bytes;
            }
            Err(err) => {
                failed = true;
                log(format!("evict: failed {}: {err}", e.target.display()));
            }
        }
    }
    if let Some((after, _)) = probe.disk(home) {
        log(format!(
            "evict: summary allocated_tree_bytes={freed} free_before_bytes={free} free_after_bytes={after} observed_free_delta_bytes={} (net filesystem change, not attributed to deleted hardlinks)",
            i128::from(after) - i128::from(free)
        ));
    }
    Collection {
        bytes: freed,
        failed,
    }
}

/// Explicit native maintenance on this machine, not a source sync or build.
/// The CLI plans (dry) unless --apply; the automatic agent default is on.
#[cfg(unix)]
pub fn command(apply: bool, prefix: &str) -> anyhow::Result<()> {
    let registry = Registry::open(prefix)?;
    let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home directory"))?;
    let (roots, skipped) = registry.snapshot()?;
    eprintln!(
        "evict: native {} registered={} rejected_receipts={} prefix={prefix:?}; unknown old mirrors are NOT enrolled or scanned",
        if apply { "apply" } else { "dry plan" },
        roots.len(),
        skipped.len()
    );
    let (free, total) = System
        .disk(&home)
        .ok_or_else(|| anyhow::anyhow!("cannot measure filesystem"))?;
    if deficit(free, total) == 0 {
        for (root, reason) in skipped {
            eprintln!("evict: skip {} ({reason:?})", root.display());
        }
        let (_, skipped) = plan(&roots, u64::MAX, DEFAULT_GRACE, &System);
        for (root, reason) in skipped {
            eprintln!("evict: skip {} ({reason:?})", root.display());
        }
        eprintln!(
            "evict: no collection needed free_bytes={free} total_bytes={total} grace_seconds={}",
            DEFAULT_GRACE.as_secs()
        );
        return Ok(());
    }
    let result = collect_report(
        &registry,
        &home,
        if apply { Mode::On } else { Mode::Dry },
        DEFAULT_GRACE,
        &System,
        &mut |line| eprintln!("{line}"),
    );
    anyhow::ensure!(
        !result.failed,
        "target collection failed; see diagnostics above"
    );
    Ok(())
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

    pub(super) fn scratch(tag: &str) -> PathBuf {
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
        reg.begin(&a).unwrap();
        let p = fake(GIB, 484 * GIB, 1_000_000);
        let (chosen, skipped) = plan(&reg.roots, u64::MAX, DEFAULT_GRACE, &p);
        assert!(chosen.is_empty());
        assert_eq!(skipped[0].1, Skip::Leased);
        reg.end(&a, 1_000_000 - 60).unwrap();
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
            evict_lock(&a).is_err(),
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
    fn mode_parsing_defaults_to_on() {
        if std::env::var_os("JUSTRUST_REMOTE_EVICT").is_none() {
            assert_eq!(Mode::from_env(), Mode::On);
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
