//! Memory governor for build hosts: admit a compiler only when the machine
//! has room for it.
//!
//! The hosted agent used to bound parallelism by `memory / 2 GiB` per job,
//! which ran 14 jobs on a 96-core, 29-GiB slot while a cold 610-unit check
//! peaked at 4.3 GB. That divisor also protected nothing: other builds of the
//! same account share the slot's memory and each applied it alone. The agent
//! now sizes jobs for the cores ([`crate::remote_agent`]) and this module
//! protects memory where a compiler is spawned (`shim.rs`), for every build
//! on the machine at once.
//!
//! How it decides, for a unit that needs `need` MB:
//!
//! - **Ledger**: every admitted unit holds an empty file in a machine-wide
//!   directory (`/dev/shm/justrust-memgov-<uid>/`), named
//!   `<shim pid>-<start ticks>-<need mb>-<build>`. Listing the directory is
//!   the whole read. The file is removed when the compiler exits. An entry
//!   whose process is gone is pruned.
//! - **Admit** when `MemAvailable >= reserve + pending + need`, where
//!   `pending` is what admitted units were promised and have not used yet
//!   (`need - resident memory of the shim's process tree`). Memory they
//!   already use is in `MemAvailable`, as is every process the governor does
//!   not manage (test binaries, build scripts, other tenants' leftovers).
//! - **Never deadlock**: a build with no admitted unit always gets one
//!   (checked without the lock). Deciders take one `flock`, so a waiter
//!   polls alone while the rest queue behind it, first come first served.
//!   The wait is bounded (`JUSTRUST_MEMGOV_MAX_WAIT_SECS`, default 120):
//!   after it the unit runs anyway. Every error admits.
//!
//! The estimate is the crate's own recorded peak (`<home>/memgov/rss/`,
//! written after each compile, rising at once and decaying slowly) plus a
//! margin, else a default by unit kind taken from two days of hosted units
//! (2026-11-26, 22,000 compiles: registry crates p99 0.5 GB, workspace
//! crates p90 1.7-2.0 GB and p99 3-4 GB, linking units highest). A unit's
//! peak includes its linker: it is the child's `ru_maxrss` over the tree.
//!
//! Nothing here is an input to a fingerprint or a cache key: it only delays
//! a spawn. On only where the build-host agent sets `JUSTRUST_MEMGOV=1`.

use crate::shim::Unit;

/// Why a unit was let through. `Fits` and `FirstOfBuild` are the governor
/// working. The rest are fail-open paths.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Admit {
    Fits,
    /// Fits, after waiting while a balloon held part of the machine's
    /// memory (hosted inflates an idle slot's and returns it under load).
    FitsAfterBalloon,
    FirstOfBuild,
    TimedOut,
    NoMeminfo,
}

/// An admitted unit as the ledger lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Holder {
    pub pid: i32,
    pub start_ticks: u64,
    pub need_mb: u64,
    pub build: u64,
}

impl Holder {
    fn file_name(&self) -> String {
        format!(
            "{}-{}-{}-{:016x}",
            self.pid, self.start_ticks, self.need_mb, self.build
        )
    }

    fn parse(name: &str) -> Option<Holder> {
        let mut it = name.split('-');
        let h = Holder {
            pid: it.next()?.parse().ok()?,
            start_ticks: it.next()?.parse().ok()?,
            need_mb: it.next()?.parse().ok()?,
            build: u64::from_str_radix(it.next()?, 16).ok()?,
        };
        it.next().is_none().then_some(h)
    }
}

/// The decision itself. `held` is each admitted unit's promise and what it
/// has realized of it (0 when unknown, which only makes this stricter).
pub(crate) fn decide(
    need_mb: u64,
    build: u64,
    reserve_mb: u64,
    available_mb: u64,
    held: &[(Holder, u64)],
) -> Option<Admit> {
    if !held.iter().any(|(h, _)| h.build == build) {
        return Some(Admit::FirstOfBuild);
    }
    let pending: u64 = held
        .iter()
        .map(|(h, used)| h.need_mb.saturating_sub(*used))
        .sum();
    (available_mb >= reserve_mb.saturating_add(pending).saturating_add(need_mb))
        .then_some(Admit::Fits)
}

/// Promises under this are small: registry crates, build scripts, small
/// local crates. Their class default is about twice what they use (flat
/// 320 MB against a measured mean of 160 to 183 MB over 3679 registry
/// units), and dozens run at once.
const SMALL_MB: u64 = 1024;

/// The second way in: memory is roomy. With a third of the machine free
/// after this unit and every large promise in full, small promises count
/// for a quarter. Without it a cold build of 589 units waited 98.6 s of
/// job time while free memory never went under 12 GB of 29.6 (run
/// 20261126-180124548-1484059): 176 registry crates stood behind each
/// other's 320 MB promises and used 160 MB.
pub(crate) fn roomy(
    need_mb: u64,
    installed_mb: u64,
    available_mb: u64,
    held: &[(Holder, u64)],
) -> bool {
    let pending: u64 = held
        .iter()
        .map(|(h, used)| {
            let left = h.need_mb.saturating_sub(*used);
            if h.need_mb < SMALL_MB { left / 4 } else { left }
        })
        .sum();
    available_mb
        >= (installed_mb / 3)
            .saturating_add(pending)
            .saturating_add(need_mb)
}

/// Memory kept free of compilers: a tenth of the machine, at least 2 GiB.
/// It absorbs what estimates miss and what the governor does not manage.
pub(crate) fn reserve_for(total_mb: u64) -> u64 {
    (total_mb / 10).max(2048)
}

/// `meta`, `rlib`, `link`, `test` or `bs`, with `-opt` for optimized
/// codegen: units of one crate in different kinds differ several-fold.
pub(crate) fn class(u: &Unit) -> String {
    let links = u.emit.split(',').any(|e| e == "link");
    let kind = if u.build_script {
        "bs"
    } else if !links {
        "meta"
    } else if u.test {
        "test"
    } else if u
        .crate_types
        .iter()
        .any(|t| matches!(t.as_str(), "bin" | "cdylib" | "dylib" | "staticlib"))
    {
        "link"
    } else {
        "rlib"
    };
    let opt = links && !matches!(u.opt_level.as_str(), "" | "0");
    format!("{kind}{}", if opt { "-opt" } else { "" })
}

/// Need of a unit nothing is recorded for, in MB.
pub(crate) fn default_need_mb(u: &Unit) -> u64 {
    let class = class(u);
    let (kind, opt) = match class.strip_suffix("-opt") {
        Some(k) => (k, true),
        None => (class.as_str(), false),
    };
    let mb = match (kind, u.local) {
        ("bs", _) => 256,
        // Measured over 48 h of hosted units (2026-11-26, peak RSS in MB):
        // build scripts p99 165, max 203. Registry rlib p90 271, p99 518,
        // meta p99 570. Local meta p90 472, p99 1858. Local rlib p90 1746,
        // p99 3267. Local test p90 1807, p99 3931, max 5092. A default
        // covers about p90. Larger units (gpui 3.4 GB, jcode_app_core tests
        // 5.1 GB) are learned after their first compile, and until then the
        // reserve and the units that use less than promised absorb them.
        (_, false) => 320,
        ("meta", true) => 768,
        ("rlib", true) => 2048,
        (_, true) => 2560,
    };
    if opt && u.local { mb + mb / 4 } else { mb }
}

/// What to set aside for a unit, in MB, from the most specific thing
/// known about it:
///
/// 1. `own`: this crate's recorded peak in this class.
/// 2. `siblings`: the same crate's peak in another class (its lib unit
///    when its test binary is new), scaled for the heavier kind.
/// 3. `source_bytes`: the size of the crate's sources. On 72 h of hosted
///    local units (2026-11-26) no debug rlib or test unit with sources
///    under 1 MB peaked above 1.0 GB, and the 2.5 GB class default made
///    three concurrent cold builds wait about 5 s each with 13 GB free.
/// 4. The class default.
pub(crate) fn need_mb(
    unit: &Unit,
    own: Option<u64>,
    siblings: &[(String, u64)],
    source_bytes: Option<u64>,
    ceiling_mb: u64,
) -> u64 {
    let margin = |r: u64| r + r / 5 + 64;
    let to = class(unit);
    let sibling = siblings
        .iter()
        .map(|(from, peak)| peak * scale_pct(from, &to) / 100)
        .max();
    let mb = match (own, sibling, source_bytes) {
        (Some(r), _, _) => margin(r),
        (None, Some(r), _) => margin(r),
        // The envelope is steep for the few large crates that set it: past
        // one and a half times the class default it promised a 2.7 GB rlib
        // 5 GB, and three first builds of it then took turns (35.8 s
        // against 25.3 s ungoverned, 2026-11-27).
        (None, None, Some(bytes)) if unit.local && !unit.build_script => {
            let default = default_need_mb(unit);
            from_source(&to, bytes).min(default + default / 2)
        }
        _ => default_need_mb(unit),
    };
    mb.min(ceiling_mb).max(64)
}

/// Percent of a crate's peak in class `from` to expect in class `to`.
/// Measured on the largest hosted crates: check to debug rlib 1.7x
/// (justrust 1543 to 2667 MB), rlib to test binary 1.55x (justrust 1863 to
/// 2883, jcode_app_core 3296 to 5091). Never scaled down.
fn scale_pct(from: &str, to: &str) -> u64 {
    let rank = |c: &str| match c.strip_suffix("-opt").unwrap_or(c) {
        "meta" => 0,
        "rlib" => 1,
        _ => 2,
    };
    let kind = match (rank(from), rank(to)) {
        (0, 1) => 200,
        (0, 2) => 300,
        (1, 2) => 160,
        _ => 100,
    };
    if to.ends_with("-opt") && !from.ends_with("-opt") {
        kind * 5 / 4
    } else {
        kind
    }
}

/// An upper envelope of peak RSS over source size (same measurement):
/// check units stayed under 320 MB + 0.5 GB per MB of source, codegen
/// units under 384 MB + 1.6 GB per MB, but for a few macro-heavy crates
/// under 0.2 MB that exceeded it by up to 0.3 GB.
fn from_source(class: &str, bytes: u64) -> u64 {
    let kb = bytes / 1024;
    let mb = if class.starts_with("meta") {
        320 + kb * 500 / 1024
    } else {
        384 + kb * 1600 / 1024
    };
    if class.ends_with("-opt") {
        mb + mb / 4
    } else {
        mb
    }
}

/// Bytes of Rust source in the crate rooted at `root`: the directory of a
/// `lib.rs`, `main.rs` or `mod.rs`, else the file and its module directory.
/// Sources reached another way (`#[path]`, `include!`, generated code) are
/// not counted. `None` when the root cannot be read.
pub(crate) fn source_bytes(root: &std::path::Path) -> Option<u64> {
    let own = std::fs::metadata(root).ok()?.len();
    let dir = root.parent()?;
    let stem = root.file_stem()?.to_str()?;
    let mut left = 20_000usize;
    Some(if matches!(stem, "lib" | "main" | "mod") {
        tree_bytes(dir, 12, &mut left).max(own)
    } else {
        own + tree_bytes(&dir.join(stem), 12, &mut left)
    })
}

fn tree_bytes(dir: &std::path::Path, depth: usize, left: &mut usize) -> u64 {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut sum = 0;
    for e in rd.flatten() {
        if *left == 0 {
            break;
        }
        *left -= 1;
        // Not following links: a link cycle or a link out of the crate.
        let Ok(t) = e.file_type() else { continue };
        if t.is_dir() && depth > 0 {
            sum += tree_bytes(&e.path(), depth - 1, left);
        } else if t.is_file() && e.path().extension().is_some_and(|x| x == "rs") {
            sum += e.metadata().map_or(0, |m| m.len());
        }
    }
    sum
}

/// The crate root among rustc's arguments.
fn root_arg(args: &[std::ffi::OsString]) -> Option<&std::path::Path> {
    args.iter().map(std::path::Path::new).find(|p| {
        p.extension().is_some_and(|x| x == "rs")
            && !p.as_os_str().as_encoded_bytes().starts_with(b"-")
    })
}

/// The recorded peak after one more compile: up at once, down a quarter of
/// the way, so one small incremental compile does not erase a cold peak.
pub(crate) fn learned(old: Option<u64>, peak_mb: u64) -> u64 {
    match old {
        Some(o) if o > peak_mb => o - (o - peak_mb) / 4,
        _ => peak_mb,
    }
}

/// What the wait loop needs from the machine, so tests supply their own.
pub(crate) trait Probe {
    /// (MemTotal, MemAvailable) in MB.
    fn meminfo(&mut self) -> Option<(u64, u64)>;
    /// For each holder: `None` if its process is gone, else the resident MB
    /// of its process tree.
    fn realized(&mut self, holders: &[Holder]) -> Vec<Option<u64>>;
    fn sleep(&mut self, d: std::time::Duration);
    fn elapsed(&self) -> std::time::Duration;
}

/// The ledger and its lock.
pub(crate) trait Ledger {
    fn list(&mut self) -> Vec<Holder>;
    fn add(&mut self, h: &Holder);
    fn remove(&mut self, h: &Holder);
    /// One decider at a time. `false`: someone else holds it.
    fn try_lock(&mut self) -> bool;
    fn unlock(&mut self);
}

pub(crate) struct Limits {
    pub max_wait: std::time::Duration,
    /// `None`: a tenth of MemTotal, at least 2 GiB.
    pub reserve_mb: Option<u64>,
    /// Memory the machine has when no balloon holds any, if known.
    pub installed_mb: Option<u64>,
}

const POLL: std::time::Duration = std::time::Duration::from_millis(50);
const RESCAN: std::time::Duration = std::time::Duration::from_millis(250);

/// Block until `me` may run, then list it. Always returns.
pub(crate) fn wait_turn(
    me: &Holder,
    limits: &Limits,
    probe: &mut dyn Probe,
    ledger: &mut dyn Ledger,
) -> Admit {
    let why = loop {
        // Without the lock: a build with nothing running is never held up
        // behind another build's waiter.
        if !ledger.list().iter().any(|h| h.build == me.build) {
            break Admit::FirstOfBuild;
        }
        if ledger.try_lock() {
            let why = decide_locked(me, limits, probe, ledger);
            // Listed before the lock is released, so the next decider
            // counts this unit.
            ledger.add(me);
            ledger.unlock();
            return why;
        }
        if probe.elapsed() >= limits.max_wait {
            break Admit::TimedOut;
        }
        probe.sleep(POLL);
    };
    ledger.add(me);
    why
}

fn decide_locked(
    me: &Holder,
    limits: &Limits,
    probe: &mut dyn Probe,
    ledger: &mut dyn Ledger,
) -> Admit {
    let mut scanned: Option<std::time::Duration> = None;
    let mut ballooned = false;
    let fits = |why: Admit, ballooned: bool| match why {
        Admit::Fits if ballooned => Admit::FitsAfterBalloon,
        other => other,
    };
    let roomy = |available: u64, held: &[(Holder, u64)]| {
        (limits.installed_mb).is_some_and(|i| roomy(me.need_mb, i, available, held))
    };
    loop {
        let Some((total, available)) = probe.meminfo() else {
            return Admit::NoMeminfo;
        };
        let reserve = limits.reserve_mb.unwrap_or_else(|| reserve_for(total));
        let holders = ledger.list();
        // Cheap: count every promise in full. Enough whenever memory is
        // not tight, and it reads nothing but the directory.
        let full: Vec<(Holder, u64)> = holders.iter().cloned().map(|h| (h, 0)).collect();
        if let Some(why) = decide(me.need_mb, me.build, reserve, available, &full) {
            return fits(why, ballooned);
        }
        if roomy(available, &full) {
            return fits(Admit::Fits, ballooned);
        }
        // Tight: look at what the holders really use, and drop the dead.
        let now = probe.elapsed();
        if scanned.is_none_or(|t| now.saturating_sub(t) >= RESCAN) {
            scanned = Some(now);
            let mut held = Vec::with_capacity(holders.len());
            for (h, used) in holders.iter().zip(probe.realized(&holders)) {
                match used {
                    Some(mb) => held.push((h.clone(), mb)),
                    None => ledger.remove(h),
                }
            }
            if let Some(why) = decide(me.need_mb, me.build, reserve, available, &held) {
                return fits(why, ballooned);
            }
            if roomy(available, &held) {
                return fits(Admit::Fits, ballooned);
            }
        }
        // Short while a fifth or more of the machine is in a balloon: the
        // wait is for the host to give it back, not for other compilers.
        ballooned |= limits.installed_mb.is_some_and(|i| total * 5 < i * 4);
        if probe.elapsed() >= limits.max_wait {
            return Admit::TimedOut;
        }
        probe.sleep(POLL);
    }
}

/// Held while the compiler runs. Dropping it frees the unit's promise.
pub(crate) struct Ticket {
    path: Option<std::path::PathBuf>,
    why: Option<Admit>,
}

impl Ticket {
    /// The shim stage the time since the previous mark is charged to.
    pub(crate) fn stage(&self) -> &'static str {
        match self.why {
            Some(Admit::TimedOut) => "wait-memory-timeout",
            Some(Admit::FitsAfterBalloon) => "wait-balloon",
            _ => "wait-memory",
        }
    }

    /// The compiler has exited: its promise is free. Dropping does the same.
    pub(crate) fn release(self) {}

    fn none() -> Ticket {
        Ticket {
            path: None,
            why: None,
        }
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        if let Some(p) = self.path.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

/// Set by the build-host agent for the builds it runs, nowhere else.
pub(crate) const ENV: &str = "JUSTRUST_MEMGOV";

/// Whether a value of [`ENV`] turns the governor on.
pub(crate) fn on(value: &str) -> bool {
    !matches!(value.trim(), "" | "0" | "off" | "false" | "no")
}

pub(crate) fn enabled() -> bool {
    std::env::var(ENV).is_ok_and(|v| on(&v))
}

/// Wait for memory before spawning the compiler for `unit`. A no-op off a
/// build host. Keep the ticket until the compiler has exited.
pub(crate) fn admit(unit: &Unit, args: &[std::ffi::OsString], run_dir: &std::path::Path) -> Ticket {
    if !enabled() {
        return Ticket::none();
    }
    linux::admit(unit, args, run_dir).unwrap_or_else(Ticket::none)
}

/// Remember what `unit` peaked at, for the next estimate.
pub(crate) fn learn(unit: &Unit) {
    if !enabled() || unit.exit != 0 || unit.max_rss_mb < 1.0 {
        return;
    }
    let Some(path) = history_path(unit) else {
        return;
    };
    let old = read_history(&path);
    let new = learned(old, unit.max_rss_mb.ceil() as u64);
    // Skip rewrites that would not move an estimate.
    if old.is_some_and(|o| o.abs_diff(new) * 20 < o.max(1)) {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, format!("{new}\n"));
}

fn history_path(unit: &Unit) -> Option<std::path::PathBuf> {
    history_file(unit, &class(unit))
}

/// History is per justrust home, so shared by every checkout and target
/// directory this machine builds a crate in.
fn history_file(unit: &Unit, class: &str) -> Option<std::path::PathBuf> {
    let name = &unit.crate_name;
    if name.is_empty()
        || name.len() > 128
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return None;
    }
    Some(
        crate::paths::home()
            .ok()?
            .join("memgov")
            .join("rss")
            .join(format!("{name}.{class}")),
    )
}

/// The same crate's recorded peaks in its other classes.
fn sibling_history(unit: &Unit) -> Vec<(String, u64)> {
    if unit.build_script {
        return Vec::new();
    }
    let own = class(unit);
    ["meta", "rlib", "link", "test"]
        .iter()
        .flat_map(|k| [k.to_string(), format!("{k}-opt")])
        .filter(|c| *c != own)
        .filter_map(|c| {
            let peak = read_history(&history_file(unit, &c)?)?;
            Some((c, peak))
        })
        .collect()
}

fn read_history(path: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(path).ok()?.trim().parse().ok()
}

mod linux {
    use super::*;
    use std::os::fd::AsRawFd;
    use std::path::PathBuf;

    struct Machine {
        started: std::time::Instant,
    }

    impl Probe for Machine {
        fn meminfo(&mut self) -> Option<(u64, u64)> {
            let m = crate::procfs::meminfo()?;
            (m.total_mb > 0).then_some((m.total_mb, m.available_mb))
        }

        fn realized(&mut self, holders: &[Holder]) -> Vec<Option<u64>> {
            // One pass over the process table: resident pages per process,
            // summed up each holder's tree (shim, compiler, cc, linker).
            let stats: Vec<_> = crate::procfs::all_pids()
                .into_iter()
                .filter_map(crate::procfs::read_stat)
                .collect();
            let page = crate::procfs::page_size();
            let by_pid: std::collections::HashMap<i32, usize> =
                stats.iter().enumerate().map(|(i, s)| (s.pid, i)).collect();
            let mut tree = vec![0u64; stats.len()];
            for s in &stats {
                // Charge each process to itself and every ancestor.
                let (mut at, mut hops) = (Some(s.pid), 0);
                while let Some(i) = at.and_then(|p| by_pid.get(&p)).copied() {
                    tree[i] += s.rss_pages;
                    at = (stats[i].ppid > 1).then_some(stats[i].ppid);
                    hops += 1;
                    if hops > 64 {
                        break;
                    }
                }
            }
            holders
                .iter()
                .map(|h| {
                    let i = *by_pid.get(&h.pid)?;
                    (stats[i].start_ticks == h.start_ticks).then(|| (tree[i] * page) >> 20)
                })
                .collect()
        }

        fn sleep(&mut self, d: std::time::Duration) {
            std::thread::sleep(d);
        }

        fn elapsed(&self) -> std::time::Duration {
            self.started.elapsed()
        }
    }

    pub(super) struct Dir {
        pub dir: PathBuf,
        pub lock: Option<std::fs::File>,
    }

    impl Ledger for Dir {
        fn list(&mut self) -> Vec<Holder> {
            let Ok(rd) = std::fs::read_dir(&self.dir) else {
                return Vec::new();
            };
            rd.filter_map(|e| Holder::parse(e.ok()?.file_name().to_str()?))
                .collect()
        }

        fn add(&mut self, h: &Holder) {
            let _ = std::fs::File::create(self.dir.join(h.file_name()));
        }

        fn remove(&mut self, h: &Holder) {
            let _ = std::fs::remove_file(self.dir.join(h.file_name()));
        }

        fn try_lock(&mut self) -> bool {
            let Ok(f) = std::fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(self.dir.join("lock"))
            else {
                // No lock file: decide alone rather than wait for nothing.
                return true;
            };
            // SAFETY: flock on a descriptor this function owns.
            let got = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0;
            if got {
                self.lock = Some(f);
            }
            got
        }

        fn unlock(&mut self) {
            // Explicitly unlock before closing: a concurrent fork() in another
            // thread duplicates this fd until exec(), keeping the flock held if
            // we only rely on close().
            if let Some(file) = self.lock.take() {
                let _ = file.unlock();
            }
        }
    }

    /// Shared by every build of this user on the machine, whatever its
    /// justrust home, and empty after a reboot.
    fn ledger_dir() -> Option<PathBuf> {
        // SAFETY: getuid has no failure mode.
        let uid = unsafe { libc::getuid() };
        let base = std::path::Path::new("/dev/shm");
        let base = if base.is_dir() {
            base.to_path_buf()
        } else {
            std::env::temp_dir()
        };
        let dir = base.join(format!("justrust-memgov-{uid}"));
        std::fs::create_dir_all(&dir).ok()?;
        Some(dir)
    }

    fn env_u64(name: &str) -> Option<u64> {
        std::env::var(name).ok()?.trim().parse().ok()
    }

    pub(super) fn admit(
        unit: &Unit,
        args: &[std::ffi::OsString],
        run_dir: &std::path::Path,
    ) -> Option<Ticket> {
        // Installed, not MemTotal: hosted balloons an idle slot down to
        // ~6 GiB and deflates it seconds after work starts. The reserve and
        // the per-unit ceiling are sized for the machine. MemAvailable is
        // what admits, so units wait out the deflation instead of racing it.
        let total = crate::procfs::installed_mb()?;
        let reserve_mb = env_u64("JUSTRUST_MEMGOV_RESERVE_MB");
        let reserve = reserve_mb.unwrap_or_else(|| reserve_for(total));
        // A unit that could never fit would only ever leave by timeout.
        let ceiling = total.saturating_sub(reserve) / 2;
        let own = history_path(unit).and_then(|p| read_history(&p));
        // The rest is read only for a unit never compiled on this machine.
        let siblings = if own.is_none() {
            sibling_history(unit)
        } else {
            Vec::new()
        };
        let source = match (own, siblings.is_empty(), unit.local) {
            (None, true, true) => root_arg(args).and_then(source_bytes),
            _ => None,
        };
        let pid = std::process::id() as i32;
        let me = Holder {
            pid,
            start_ticks: crate::procfs::read_stat(pid)?.start_ticks,
            need_mb: need_mb(unit, own, &siblings, source, ceiling),
            build: {
                use std::hash::{Hash, Hasher};
                let mut h = std::collections::hash_map::DefaultHasher::new();
                run_dir.hash(&mut h);
                h.finish()
            },
        };
        let dir = ledger_dir()?;
        let path = dir.join(me.file_name());
        let limits = Limits {
            max_wait: std::time::Duration::from_secs(
                env_u64("JUSTRUST_MEMGOV_MAX_WAIT_SECS").unwrap_or(120),
            ),
            reserve_mb,
            installed_mb: Some(total),
        };
        let mut probe = Machine {
            started: std::time::Instant::now(),
        };
        let why = wait_turn(&me, &limits, &mut probe, &mut Dir { dir, lock: None });
        Some(Ticket {
            path: Some(path),
            why: Some(why),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn holder(pid: i32, need_mb: u64, build: u64) -> Holder {
        Holder {
            pid,
            start_ticks: 7,
            need_mb,
            build,
        }
    }

    /// A machine whose free memory follows a script: `avail[i]` is what the
    /// i-th read of meminfo returns (the last value repeats).
    struct Fake {
        total: u64,
        avail: Vec<Option<u64>>,
        reads: usize,
        /// pid -> resident MB. A listed pid missing here is a dead process.
        rss: std::collections::HashMap<i32, u64>,
        clock: Duration,
        scans: usize,
    }

    impl Fake {
        fn new(total: u64, avail: &[u64]) -> Fake {
            Fake {
                total,
                avail: avail.iter().map(|a| Some(*a)).collect(),
                reads: 0,
                rss: Default::default(),
                clock: Duration::ZERO,
                scans: 0,
            }
        }
    }

    impl Probe for Fake {
        fn meminfo(&mut self) -> Option<(u64, u64)> {
            let i = self.reads.min(self.avail.len() - 1);
            self.reads += 1;
            self.avail[i].map(|a| (self.total, a))
        }
        fn realized(&mut self, holders: &[Holder]) -> Vec<Option<u64>> {
            self.scans += 1;
            holders
                .iter()
                .map(|h| self.rss.get(&h.pid).copied())
                .collect()
        }
        fn sleep(&mut self, d: Duration) {
            self.clock += d;
        }
        fn elapsed(&self) -> Duration {
            self.clock
        }
    }

    #[derive(Default)]
    struct Mem {
        held: Vec<Holder>,
        /// Lock attempts that fail before one succeeds.
        busy: usize,
        locked: bool,
        /// A holder that exits (leaves the ledger) at this many `list` calls.
        leaves: Option<(usize, i32)>,
        lists: usize,
    }

    impl Ledger for Mem {
        fn list(&mut self) -> Vec<Holder> {
            self.lists += 1;
            if let Some((at, pid)) = self.leaves
                && self.lists >= at
            {
                self.held.retain(|h| h.pid != pid);
            }
            self.held.clone()
        }
        fn add(&mut self, h: &Holder) {
            self.held.push(h.clone());
        }
        fn remove(&mut self, h: &Holder) {
            self.held.retain(|x| x != h);
        }
        fn try_lock(&mut self) -> bool {
            if self.busy > 0 {
                self.busy -= 1;
                return false;
            }
            self.locked = true;
            true
        }
        fn unlock(&mut self) {
            self.locked = false;
        }
    }

    fn limits(secs: u64) -> Limits {
        Limits {
            max_wait: Duration::from_secs(secs),
            reserve_mb: None,
            installed_mb: Some(SLOT_MB),
        }
    }

    const SLOT_MB: u64 = 29_577; // the hosted slot: MemTotal 30287028 kB

    #[test]
    fn reserve_is_a_tenth_and_at_least_two_gib() {
        assert_eq!(reserve_for(SLOT_MB), 2957);
        assert_eq!(reserve_for(8_000), 2048);
    }

    #[test]
    fn a_build_with_nothing_running_always_gets_one_compiler() {
        // No memory at all, another build holding the lock forever.
        let mut probe = Fake::new(SLOT_MB, &[0]);
        let mut ledger = Mem {
            held: vec![holder(1, 4000, 0xa)],
            busy: usize::MAX,
            ..Default::default()
        };
        let me = holder(2, 4000, 0xb);
        let why = wait_turn(&me, &limits(120), &mut probe, &mut ledger);
        assert_eq!(why, Admit::FirstOfBuild);
        assert_eq!(probe.clock, Duration::ZERO, "no wait");
        assert!(ledger.held.contains(&me), "listed for the next decider");
    }

    #[test]
    fn admits_at_once_when_every_promise_fits_without_reading_processes() {
        // 20 GB free, reserve 2957, 10 units promised 320 MB each.
        let mut probe = Fake::new(SLOT_MB, &[20_000]);
        let mut held: Vec<Holder> = (0..10).map(|i| holder(100 + i, 320, 0xa)).collect();
        held.push(holder(50, 500, 0xb));
        let mut ledger = Mem {
            held,
            ..Default::default()
        };
        let me = holder(2, 2048, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(120), &mut probe, &mut ledger),
            Admit::Fits
        );
        assert_eq!(probe.scans, 0);
        assert_eq!(probe.clock, Duration::ZERO);
        assert!(!ledger.locked && ledger.held.contains(&me));
    }

    #[test]
    fn counts_other_builds_promises_not_only_its_own() {
        // Another build was just promised 3 x 2 GB it has not touched yet,
        // this one 100 MB: 9200 - 2957 - 6100 leaves 143 MB, not 2 GB.
        let held = [
            holder(10, 2000, 0xa),
            holder(11, 2000, 0xa),
            holder(12, 2000, 0xa),
            holder(20, 100, 0xb),
        ];
        let full: Vec<(Holder, u64)> = held.iter().cloned().map(|h| (h, 0)).collect();
        assert_eq!(decide(2000, 0xb, 2957, 9200, &full), None);
        assert_eq!(decide(144, 0xb, 2957, 9200, &full), None);
        assert_eq!(decide(143, 0xb, 2957, 9200, &full), Some(Admit::Fits));
        // Once they use what they were promised, MemAvailable already
        // shows it and the promise is no longer counted twice.
        let used: Vec<(Holder, u64)> = held.iter().cloned().map(|h| (h, 2500)).collect();
        assert_eq!(decide(2000, 0xb, 2957, 9000, &used), Some(Admit::Fits));
    }

    #[test]
    fn waits_while_memory_is_short_and_runs_when_it_frees() {
        // Short for the first 40 polls (2s), then a compiler exits.
        let mut avail = vec![4000u64; 40];
        avail.push(12_000);
        let mut probe = Fake::new(SLOT_MB, &avail);
        probe.rss.insert(50, 900);
        let mut ledger = Mem {
            held: vec![holder(50, 1000, 0xb)],
            ..Default::default()
        };
        let me = holder(2, 2048, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(120), &mut probe, &mut ledger),
            Admit::Fits
        );
        assert_eq!(probe.clock, POLL * 40);
        // Process table read every 250 ms of waiting, not every poll.
        assert!(probe.scans >= 8 && probe.scans <= 9, "{}", probe.scans);
    }

    #[test]
    fn a_wait_for_the_slots_balloon_to_deflate_is_named_as_that() {
        // An idle slot ballooned to 5.5 GB: MemTotal 5513, 4400 available.
        // The host returns the memory after 3 s (60 polls).
        let mut avail = vec![4400u64; 60];
        avail.push(20_000);
        let mut probe = Fake::new(5513, &avail);
        probe.rss.insert(50, 300);
        let mut ledger = Mem {
            held: vec![holder(50, 400, 0xb)],
            ..Default::default()
        };
        // Reserve is 2048 of the ballooned total: a 2.5 GB unit must wait.
        let me = holder(2, 2500, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(120), &mut probe, &mut ledger),
            Admit::FitsAfterBalloon
        );
        assert_eq!(probe.clock, POLL * 60);
        // A small unit fits in what the balloon left and is not held up.
        let mut probe = Fake::new(5513, &[4400]);
        probe.rss.insert(50, 300);
        probe.rss.insert(2, 2400);
        let small = holder(3, 300, 0xb);
        assert_eq!(
            wait_turn(&small, &limits(120), &mut probe, &mut ledger),
            Admit::Fits
        );
        assert_eq!(probe.clock, Duration::ZERO);
    }

    #[test]
    fn small_units_are_not_held_while_a_third_of_the_machine_is_free() {
        // The production case: 60 registry crates in flight, each promised
        // 320 MB, 20 GB of 29.6 available. Counted in full they do not fit.
        let small: Vec<(Holder, u64)> = (0..60).map(|i| (holder(100 + i, 320, 0xb), 0)).collect();
        assert_eq!(decide(320, 0xb, 2957, 20_000, &small), None);
        assert!(roomy(320, SLOT_MB, 20_000, &small));
        let mut probe = Fake::new(SLOT_MB, &[20_000]);
        let mut ledger = Mem {
            held: small.iter().map(|(h, _)| h.clone()).collect(),
            ..Default::default()
        };
        assert_eq!(
            wait_turn(&holder(2, 320, 0xb), &limits(120), &mut probe, &mut ledger),
            Admit::Fits
        );
        assert_eq!((probe.clock, probe.scans), (Duration::ZERO, 0));
        // Not once memory is no longer roomy: a third is 9859 MB.
        assert!(!roomy(320, SLOT_MB, 14_900, &small));
        assert!(roomy(320, SLOT_MB, 15_000, &small));
        // Large promises count in full: three 3 GB units starting leave no
        // room for a fourth at 20 GB available, small ones or not.
        let large: Vec<(Holder, u64)> = (0..3).map(|i| (holder(200 + i, 3000, 0xb), 0)).collect();
        assert!(!roomy(3000, SLOT_MB, 20_000, &large));
        assert!(roomy(320, SLOT_MB, 20_000, &large));
        // What they have realized is no longer pending.
        let grown: Vec<(Holder, u64)> = large.iter().map(|(h, _)| (h.clone(), 2500)).collect();
        assert!(roomy(3000, SLOT_MB, 14_400, &grown));
    }

    #[test]
    fn the_wait_is_bounded_and_fails_open() {
        let mut probe = Fake::new(SLOT_MB, &[1000]);
        probe.rss.insert(50, 10);
        let mut ledger = Mem {
            held: vec![holder(50, 1000, 0xb)],
            ..Default::default()
        };
        let me = holder(2, 2048, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(3), &mut probe, &mut ledger),
            Admit::TimedOut
        );
        assert_eq!(probe.clock, Duration::from_secs(3));
        assert!(ledger.held.contains(&me) && !ledger.locked);
    }

    #[test]
    fn waiting_behind_another_decider_is_bounded_too() {
        let mut probe = Fake::new(SLOT_MB, &[20_000]);
        let mut ledger = Mem {
            held: vec![holder(50, 1000, 0xb)],
            busy: usize::MAX,
            ..Default::default()
        };
        let me = holder(2, 64, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(2), &mut probe, &mut ledger),
            Admit::TimedOut
        );
        assert_eq!(probe.clock, Duration::from_secs(2));
    }

    #[test]
    fn a_queued_unit_runs_when_its_build_has_nothing_left_running() {
        // Queued behind another decider. Its build's only compiler exits.
        let mut probe = Fake::new(SLOT_MB, &[0]);
        let mut ledger = Mem {
            held: vec![holder(50, 1000, 0xb), holder(60, 1000, 0xa)],
            busy: usize::MAX,
            leaves: Some((5, 50)),
            ..Default::default()
        };
        let me = holder(2, 4000, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(120), &mut probe, &mut ledger),
            Admit::FirstOfBuild
        );
        assert_eq!(probe.clock, POLL * 4);
    }

    #[test]
    fn a_dead_holder_is_pruned_and_stops_counting() {
        // pid 77 was killed with its promise listed. 5 GB free is short
        // with it (2957 + 4000 + 100 + 1000) and enough without.
        let mut probe = Fake::new(SLOT_MB, &[5000]);
        probe.rss.insert(50, 0);
        let dead = holder(77, 4000, 0xa);
        let mut ledger = Mem {
            held: vec![dead.clone(), holder(50, 100, 0xb)],
            ..Default::default()
        };
        let me = holder(2, 1000, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(120), &mut probe, &mut ledger),
            Admit::Fits
        );
        assert!(!ledger.held.contains(&dead));
        assert_eq!(probe.clock, Duration::ZERO);
    }

    #[test]
    fn a_dead_holder_of_its_own_build_does_not_block_the_first_unit() {
        let mut probe = Fake::new(SLOT_MB, &[0]);
        let mut ledger = Mem {
            held: vec![holder(77, 4000, 0xb)],
            ..Default::default()
        };
        let me = holder(2, 1000, 0xb);
        assert_eq!(
            wait_turn(&me, &limits(120), &mut probe, &mut ledger),
            Admit::FirstOfBuild
        );
        assert_eq!(probe.clock, Duration::ZERO);
    }

    #[test]
    fn unreadable_meminfo_admits() {
        let mut probe = Fake::new(SLOT_MB, &[0]);
        probe.avail = vec![None];
        let mut ledger = Mem {
            held: vec![holder(50, 1000, 0xb)],
            ..Default::default()
        };
        assert_eq!(
            wait_turn(&holder(2, 1000, 0xb), &limits(120), &mut probe, &mut ledger),
            Admit::NoMeminfo
        );
    }

    #[test]
    fn ninety_six_jobs_of_registry_crates_fit_the_slot_and_big_units_do_not_pile_up() {
        // An idle slot has about 25.5 GB available. Units are admitted one
        // after another before any has used memory (the worst case).
        let admit_all = |need: u64| {
            let mut held: Vec<(Holder, u64)> = vec![(holder(1, need, 0xa), 0)];
            while decide(need, 0xa, reserve_for(SLOT_MB), 25_500, &held).is_some() {
                held.push((holder(held.len() as i32 + 1, need, 0xa), 0));
            }
            held.len()
        };
        // Learned registry crates (p50 150 MB recorded): 92 at once even
        // before any of them has touched its memory.
        let reg = unit(false, "dep-info,metadata", &["lib"], "0", false);
        assert!(admit_all(need_mb(&reg, Some(150), &[], None, 13_000)) >= 92);
        // Unknown registry crates at the default.
        assert_eq!(admit_all(320), 70);
        // Workspace test binaries nothing is known about: 8 at once.
        assert_eq!(admit_all(2560), 8);
        // A first build of a workspace of small crates (60 KB of source
        // each) is not held to that: 47 of its test binaries at once.
        let test = unit(true, "dep-info,link", &["lib"], "0", true);
        let small = need_mb(&test, None, &[], Some(60 << 10), 13_000);
        assert_eq!(small, 477);
        assert_eq!(admit_all(small), 47);
    }

    #[test]
    fn an_unseen_unit_is_sized_from_what_is_known_about_its_crate() {
        let test = unit(true, "dep-info,link", &["lib"], "0", true);
        let rlib = unit(true, "dep-info,metadata,link", &["lib"], "0", false);
        let meta = unit(true, "dep-info,metadata", &["lib"], "0", false);
        let sib = |c: &str, mb| vec![(c.to_string(), mb)];
        // Its own record wins over everything.
        assert_eq!(
            need_mb(&test, Some(1000), &sib("rlib", 9000), Some(1), 13_000),
            1264
        );
        // The lib's peak, scaled to a test binary: justrust's rlib peaked
        // at 1863 MB and its test binary at 2883.
        let seeded = need_mb(&test, None, &sib("rlib", 1863), Some(1), 13_000);
        assert_eq!(seeded, 3640);
        assert!(seeded > 2883);
        // A check peak seeds a build, and a heavier sibling is not scaled.
        assert_eq!(need_mb(&rlib, None, &sib("meta", 500), None, 13_000), 1264);
        assert_eq!(need_mb(&meta, None, &sib("test", 1000), None, 13_000), 1264);
        assert_eq!(
            need_mb(&meta, None, &sib("rlib-opt", 1000), None, 13_000),
            1264
        );
        // Source size: the measured peaks sit under these.
        let src = |u: &Unit, mb: u64| need_mb(u, None, &[], Some(mb << 20), 13_000);
        assert_eq!(src(&meta, 0), 320);
        assert_eq!(src(&rlib, 0), 384);
        // Large crates: one and a half class defaults, which covers
        // justrust (rlib 2667 MB, test binary 2883) and leaves
        // jcode_app_core's 5.1 GB test binary to the reserve.
        assert_eq!((src(&meta, 4), src(&rlib, 4)), (1152, 3072));
        assert_eq!((src(&test, 7), src(&test, 40)), (3840, 3840));
        // Registry crates and build scripts keep their class default.
        let reg = unit(false, "dep-info,metadata,link", &["lib"], "0", false);
        assert_eq!(need_mb(&reg, None, &[], Some(9 << 20), 13_000), 320);
        // Nothing known at all.
        assert_eq!(need_mb(&test, None, &[], None, 13_000), 2560);
    }

    #[test]
    fn source_size_counts_the_crate_tree_or_a_single_target_file() {
        let dir = std::env::temp_dir().join(format!("jr-memgov-src-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let w = |rel: &str, bytes: usize| {
            let p = dir.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, vec![b'x'; bytes]).unwrap();
            p
        };
        let lib = w("src/lib.rs", 100);
        w("src/a/b.rs", 1000);
        w("src/notes.txt", 5000);
        let it = w("tests/it.rs", 30);
        w("tests/it/helper.rs", 7);
        w("tests/other.rs", 900);
        assert_eq!(source_bytes(&lib), Some(1100));
        assert_eq!(source_bytes(&it), Some(37));
        assert_eq!(source_bytes(&dir.join("src/gone.rs")), None);
        let _ = std::fs::remove_dir_all(&dir);
        let args: Vec<std::ffi::OsString> = [
            "--crate-name",
            "x",
            "--edition=2021",
            "src/lib.rs",
            "-C",
            "x.rs",
        ]
        .iter()
        .map(Into::into)
        .collect();
        assert_eq!(root_arg(&args), Some(std::path::Path::new("src/lib.rs")));
    }

    fn unit(local: bool, emit: &str, types: &[&str], opt: &str, test: bool) -> Unit {
        Unit {
            crate_name: "x".into(),
            local,
            emit: emit.into(),
            crate_types: types.iter().map(|t| t.to_string()).collect(),
            opt_level: opt.into(),
            test,
            ..Default::default()
        }
    }

    #[test]
    fn classes_and_defaults_follow_the_unit_kind() {
        let meta = unit(false, "dep-info,metadata", &["lib"], "0", false);
        let rlib = unit(true, "dep-info,metadata,link", &["lib"], "0", false);
        let bin = unit(true, "dep-info,link", &["bin"], "3", false);
        let test = unit(true, "dep-info,link", &["lib"], "0", true);
        let mut bs = unit(true, "dep-info,link", &["bin"], "0", false);
        bs.build_script = true;
        assert_eq!(class(&meta), "meta");
        assert_eq!(class(&rlib), "rlib");
        assert_eq!(class(&bin), "link-opt");
        assert_eq!(class(&test), "test");
        assert_eq!(class(&bs), "bs");
        assert_eq!(default_need_mb(&meta), 320);
        assert_eq!(default_need_mb(&rlib), 2048);
        assert_eq!(default_need_mb(&bin), 3200);
        assert_eq!(default_need_mb(&test), 2560);
        assert_eq!(default_need_mb(&bs), 256);
    }

    #[test]
    fn the_estimate_follows_the_recorded_peak_with_a_margin() {
        let x = unit(true, "dep-info,link", &["lib"], "0", true);
        assert_eq!(need_mb(&x, Some(1000), &[], None, 13_000), 1264);
        // Never more than half of what the machine can give a build.
        assert_eq!(need_mb(&x, Some(40_000), &[], None, 13_000), 13_000);
        // Peaks rise at once and fall slowly.
        assert_eq!(learned(None, 800), 800);
        assert_eq!(learned(Some(800), 3000), 3000);
        assert_eq!(learned(Some(3000), 200), 2300);
    }

    #[test]
    fn ledger_names_round_trip_and_ignore_other_files() {
        let h = holder(4321, 2048, 0xdead_beef);
        assert_eq!(Holder::parse(&h.file_name()), Some(h));
        assert_eq!(Holder::parse("lock"), None);
        assert_eq!(Holder::parse("1-2-3"), None);
        assert_eq!(Holder::parse("1-2-3-zz"), None);
        assert_eq!(Holder::parse("1-2-3-ff-extra"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn the_directory_ledger_lists_adds_removes_and_locks_exclusively() {
        let dir = std::env::temp_dir().join(format!("jr-memgov-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut a = linux::Dir {
            dir: dir.clone(),
            lock: None,
        };
        let mut b = linux::Dir {
            dir: dir.clone(),
            lock: None,
        };
        let h = holder(9, 512, 3);
        a.add(&h);
        assert!(a.try_lock());
        assert!(!b.try_lock(), "one decider at a time");
        assert_eq!(b.list(), vec![h.clone()], "the lock file is not a holder");
        a.unlock();
        assert!(b.try_lock());
        b.remove(&h);
        assert!(a.list().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
