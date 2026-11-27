//! The rustc shim. Cargo runs it as `RUSTC` (`~/.justrust/bin/shim/rustc`) while
//! a recorded run is active. For each compilation unit it records wall time,
//! CPU time, peak memory, when metadata (`.rmeta`) became available, and, when
//! it is safe, rustc's own per-pass timings.
//!
//! Incremental-cache safety: rustc keys its incremental cache on whether
//! unstable options are allowed (`RUSTC_BOOTSTRAP`), but not on `-Ztime-passes`.
//! So pass timing is only ever added to units that already run with
//! `RUSTC_BOOTSTRAP=1` (for example via the Jcode Desktop parallel-frontend
//! wrapper) or on a pinned nightly. It is off by default: `-Ztime-passes`
//! made a type-error `check -p jcode-app-core --tests` ~0.4 s (10%) slower
//! (FINDINGS 41). `JUSTRUST_PASSES=on` turns it on where it is cache-safe,
//! `JUSTRUST_PASSES=always` everywhere, at the cost of one cold incremental
//! rebuild whenever the mode flips.
//!
//! Default phase timing: on a jr compiler with the phase ledger
//! (`phase_ledger_path`), every unit gets its per-phase split without
//! `-Ztime-passes`. The compiler sums phase durations in memory and writes one
//! file at exit, named by `JUSTRUST_RUSTC_PHASES_FILE`. That is an environment
//! variable, not a compiler option, so no cache key moves and dependency units
//! stay eligible for the depcache. `JUSTRUST_PASSES=0` turns it off.

use crate::paths;
use serde::Serialize;
use std::ffi::{OsStr, OsString};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

#[derive(Debug, Default, Serialize, serde::Deserialize, Clone)]
pub struct Unit {
    pub crate_name: String,
    pub package: String,
    pub version: String,
    pub manifest_dir: String,
    pub crate_types: Vec<String>,
    pub test: bool,
    pub build_script: bool,
    /// Compiled from a local path (workspace or path dependency), not the registry.
    pub local: bool,
    pub emit: String,
    pub opt_level: String,
    pub incremental: bool,
    /// For incremental compiles, whether `-C incremental=<dir>` already
    /// contained a prior `s-*` session for this crate before rustc started.
    /// Distinguishes an unseeded/empty incremental dir from rustc rejecting a
    /// prior session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub incr_prior_session: Option<bool>,
    pub frontend_threads: Option<u32>,
    pub start: f64,
    pub end: f64,
    pub wall: f64,
    pub user: f64,
    pub sys: f64,
    pub max_rss_mb: f64,
    /// Seconds after start when rustc reported the `.rmeta` artifact.
    pub rmeta_secs: Option<f64>,
    pub exit: i32,
    pub passes: Vec<Pass>,
    /// Source or absence of compiler phase telemetry for a real compile.
    /// Empty on cache hits and recordings from older clients.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub phase_coverage: String,
    /// Shared dependency cache outcome: "hit", "miss" (compiled, stored when
    /// it succeeded), or empty when the unit was not eligible (local,
    /// incremental, disabled with `JUSTRUST_DEPCACHE=0`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub depcache: String,
    /// On a hit, how long the original compile took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depcache_saved_secs: Option<f64>,
    /// Hit path including key computation and restore, excluding recorder writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_hit_overhead_secs: Option<f64>,
    /// On a miss: keyed env vars cargo does not set itself (user env or a
    /// cargo `[env]`), which keep this unit's key from matching the store.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyed_env: Vec<String>,
    /// On a miss: which part of the key moved (`depcache_why.rs`), e.g.
    /// `dependency: serde`, `evicted`, `first-seen: package`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub depcache_why: String,
    /// The compiler's reuse ledger: what incremental compilation did with
    /// each query and codegen unit (`reuse.rs`). Empty without a capable
    /// compiler.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reuse: Vec<crate::reuse::Row>,
    /// Seconds the shim spent before `start` (the compiler spawn, or the
    /// cache tier that hit): argument rewriting, cache keys and every cache
    /// tier that missed. Cargo holds a job slot for all of it, and none of
    /// it is inside `wall`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shim_pre_secs: Option<f64>,
    /// Seconds the shim spent after `end` before it recorded the unit
    /// (storing the outputs in the caches). Absent on cache hits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shim_post_secs: Option<f64>,
    /// Where `shim_pre_secs` and `shim_post_secs` went: stages of at least
    /// a millisecond, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shim_stages: Vec<ShimStage>,
}

#[derive(Debug, Serialize, serde::Deserialize, Clone, PartialEq)]
pub struct ShimStage {
    pub name: String,
    pub secs: f64,
}

/// Wall-clock split of the shim's own work around one compiler invocation.
struct Stages {
    last: std::time::Instant,
    pre: f64,
    post: f64,
    after: bool,
    rows: Vec<ShimStage>,
}

impl Stages {
    fn new(started: std::time::Instant) -> Self {
        Stages {
            last: started,
            pre: 0.0,
            post: 0.0,
            after: false,
            rows: Vec::new(),
        }
    }

    /// Charge the time since the previous mark to `name`.
    fn mark(&mut self, name: &str) {
        let now = std::time::Instant::now();
        let secs = now.duration_since(self.last).as_secs_f64();
        self.last = now;
        if self.after {
            self.post += secs;
        } else {
            self.pre += secs;
        }
        if secs < 0.001 {
            return;
        }
        match self.rows.iter_mut().find(|r| r.name == name) {
            Some(r) => r.secs += secs,
            None => self.rows.push(ShimStage {
                name: name.to_owned(),
                secs,
            }),
        }
    }

    /// The compiler ran since the last mark: that time is the unit's own
    /// `wall`, not the shim's.
    fn resume_after(&mut self) {
        self.last = std::time::Instant::now();
        self.after = true;
    }

    fn apply(&self, unit: &mut Unit) {
        unit.shim_pre_secs = Some(self.pre);
        unit.shim_post_secs = self.after.then_some(self.post);
        unit.shim_stages = self.rows.clone();
    }
}

#[derive(Debug, Serialize, serde::Deserialize, Clone)]
pub struct Pass {
    pub name: String,
    pub secs: f64,
    pub rss_end_mb: f64,
    /// How many times the phase ran, when the compiler's phase ledger
    /// reported it (zero from `-Ztime-passes`).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub calls: u64,
}

fn is_zero(n: &u64) -> bool {
    *n == 0
}

fn real_rustc() -> PathBuf {
    if let Some(p) = std::env::var_os("JUSTRUST_REAL_RUSTC") {
        return PathBuf::from(p);
    }
    let me = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_default();
    paths::find_on_path("rustc", &me).unwrap_or_else(|| PathBuf::from("rustc"))
}

pub fn main() -> ! {
    let started = std::time::Instant::now();
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let real = real_rustc();
    share_slot_incremental(&mut args);
    args.extend(cache_proc_macro_args(&real, &args));
    args.extend(release_frontend_threads_args(&real, &args));
    jr_iface_hash_env(&real);
    let run_dir = std::env::var_os("JUSTRUST_RUN_DIR").map(PathBuf::from);
    let is_unit =
        std::env::var_os("CARGO_CRATE_NAME").is_some() && args.iter().any(|a| a == "--crate-name");
    let Some(run_dir) = run_dir.filter(|_| is_unit) else {
        if is_unit && crate::nolocal::disabled() {
            eprintln!("{}", crate::nolocal::refusal("a local rustc"));
            std::process::exit(1);
        }
        exec(&real, &args);
    };
    if crate::nolocal::disabled() {
        eprintln!("{}", crate::nolocal::refusal("a local rustc"));
        std::process::exit(1);
    }
    match run_unit(&real, &args, &run_dir, started) {
        Some(code) => std::process::exit(code),
        None => exec(&real, &args),
    }
}

/// Point a build slot's `-C incremental=<root>/justrust-slots/<n>/<profile>/incremental`
/// at the shared `<root>/<profile>/incremental`. Slots share one source tree,
/// so another slot's newest session is the best previous state; with private
/// dirs every slot change was a cold start (214s of 1246s cold local compile
/// on 2026-11-18). rustc locks and finalizes session dirs, so concurrent
/// sessions are safe, and the path is neither a cargo fingerprint input nor an
/// output. `JUSTRUST_SHARED_INCREMENTAL=0` keeps private dirs.
fn share_slot_incremental(args: &mut [OsString]) {
    if std::env::var_os("JUSTRUST_SHARED_INCREMENTAL").is_some_and(|v| v == "0") {
        return;
    }
    let base = std::env::var_os("JUSTRUST_SHARED_INCREMENTAL_DIR");
    share_slot_incremental_in(args, base.as_deref());
}

/// `share_slot_incremental` with the hosted shared base passed in.
fn share_slot_incremental_in(args: &mut [OsString], base: Option<&std::ffi::OsStr>) {
    for i in 0..args.len() {
        let a = args[i].to_string_lossy().into_owned();
        let (prefix, path) = if let Some(p) = a.strip_prefix("-Cincremental=") {
            ("-Cincremental=", p.to_owned())
        } else if a == "-C"
            && let Some(next) = args.get(i + 1).map(|n| n.to_string_lossy().into_owned())
            && let Some(p) = next.strip_prefix("incremental=")
        {
            let Some(shared) = shared_incremental_dir(std::path::Path::new(p), base) else {
                return;
            };
            args[i + 1] = format!("incremental={}", shared.display()).into();
            return;
        } else {
            continue;
        };
        if let Some(shared) = shared_incremental_dir(std::path::Path::new(&path), base) {
            args[i] = format!("{prefix}{}", shared.display()).into();
        }
        return;
    }
}

/// Redirect `-C incremental=...` to a shared incremental directory:
/// 1. If `base` (`JUSTRUST_SHARED_INCREMENTAL_DIR`, e.g. hosted canonical repo
///    guard) is set, map `<anything>/<profile>/incremental` to
///    `<base>/<profile>/incremental` so all worktrees of the same repo share
///    one warm incremental cache.
/// 2. Otherwise map `<root>/justrust-slots/<n>/<rest...>/incremental` ->
///    `<root>/<rest...>/incremental`.
fn shared_incremental_dir(p: &std::path::Path, base: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let comps: Vec<_> = p.components().collect();
    if comps.last()?.as_os_str() != "incremental" {
        return None;
    }
    if let Some(base) = base
        && !base.is_empty()
        && comps.len() >= 2
    {
        let profile = comps[comps.len() - 2].as_os_str();
        let out = PathBuf::from(base).join(profile).join("incremental");
        let _ = std::fs::create_dir_all(&out);
        return Some(out);
    }
    let at = comps
        .iter()
        .position(|c| c.as_os_str() == "justrust-slots")?;
    let slot = comps.get(at + 1)?.as_os_str().to_str()?;
    if slot.is_empty() || !slot.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if comps.len() < at + 4 {
        return None;
    }
    let mut out: PathBuf = comps[..at].iter().collect();
    out.extend(&comps[at + 2..]);
    Some(out)
}

/// Check whether `-C incremental=<dir>` already contains a prior `s-*`
/// session directory for `crate_name` before rustc runs.
fn prior_incr_session(args: &[OsString], crate_name: &str) -> Option<bool> {
    let mut dir: Option<PathBuf> = None;
    for i in 0..args.len() {
        let a = args[i].to_string_lossy();
        if let Some(p) = a.strip_prefix("-Cincremental=") {
            dir = Some(PathBuf::from(p));
            break;
        }
        if a == "-C"
            && let Some(next) = args.get(i + 1)
            && let Some(p) = next.to_string_lossy().strip_prefix("incremental=")
        {
            dir = Some(PathBuf::from(p));
            break;
        }
    }
    let dir = dir?;
    let prefix = format!("{crate_name}-");
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Some(false),
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with(&prefix) {
            continue;
        }
        if let Ok(sub) = std::fs::read_dir(entry.path()) {
            for s in sub.flatten() {
                if s.file_name().to_string_lossy().starts_with("s-") {
                    return Some(true);
                }
            }
        }
    }
    Some(false)
}

fn exec(real: &PathBuf, args: &[OsString]) -> ! {
    let err = crate::sys::exec(Command::new(real).args(args));
    eprintln!("justrust: failed to exec {}: {err}", real.display());
    std::process::exit(127);
}

/// `-Zcache-proc-macros` for incremental units on our compiler.
/// Default-on requires enforced input tracking, not merely an observational
/// query. Libc interposition misses environment enumeration, direct syscalls,
/// inherited descriptors, and some clock reads. No currently shipped compiler
/// advertises the enforced capability. Explicit opt-in remains experimental.
/// `JUSTRUST_CACHE_PROC_MACROS=0/off/false/no` disables it,
/// `1/on/true/yes` forces it. The flag is part of rustc's incremental key,
/// independent of recording.
fn cache_proc_macro_args(real: &std::path::Path, args: &[OsString]) -> Option<OsString> {
    // The variable is inherited by nested cargo invocations (build scripts,
    // xtasks) that may run another rustc: also require that the compiler
    // really is a jr toolchain's (`.../toolchains/jr-*/bin/rustc`).
    let is_jr = real
        .parent()
        .and_then(|b| b.parent())
        .and_then(|d| d.file_name())
        .is_some_and(|n| n.to_string_lossy().starts_with("jr-"));
    if !is_jr || std::env::var_os("JUSTRUST_TOOLCHAIN_JR").is_none() {
        return None;
    }
    let enabled = real
        .parent()
        .and_then(|b| b.parent())
        .is_some_and(proc_macro_cache_enabled);
    if !enabled {
        return None;
    }
    let s = |a: &OsString| a.to_string_lossy().into_owned();
    let incremental = args
        .windows(2)
        .any(|w| s(&w[0]) == "-C" && s(&w[1]).starts_with("incremental="))
        || args.iter().any(|a| s(a).starts_with("-Cincremental="));
    let already = args.iter().any(|a| s(a).contains("cache-proc-macros"));
    (incremental && !already).then(|| OsString::from("-Zcache-proc-macros"))
}

pub(crate) fn proc_macro_cache_enabled(toolchain: &std::path::Path) -> bool {
    match std::env::var("JUSTRUST_CACHE_PROC_MACROS").as_deref() {
        Ok("1" | "on" | "true" | "yes") => true,
        Ok("0" | "off" | "false" | "no") => false,
        _ => proc_macro_inputs_enforced(toolchain),
    }
}

/// Whether the compiler advertises enforced proc-macro input isolation.
/// Observational `jr_proc_macro_inputs` tracking is deliberately insufficient.
/// A distinct marker prevents old probe results enabling unsafe reuse.
fn proc_macro_inputs_enforced(toolchain: &std::path::Path) -> bool {
    let marker = toolchain.join(".jr-pm-inputs-enforced-v1");
    if let Ok(v) = std::fs::read_to_string(&marker) {
        return v.trim() == "yes";
    }
    let found = std::fs::read_dir(toolchain.join("lib"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with("librustc_driver") && n.ends_with(".so")
        })
        .any(|e| {
            std::fs::read(e.path()).is_ok_and(|b| {
                memchr::memmem::find(&b, b"jr_proc_macro_inputs_enforced_v1").is_some()
            })
        });
    let tmp = toolchain.join(format!(".jr-pm-inputs-enforced-v1.{}", std::process::id()));
    if std::fs::write(&tmp, if found { "yes\n" } else { "no\n" }).is_ok() {
        let _ = std::fs::rename(&tmp, &marker);
    }
    found
}

/// Whether the toolchain's compiler has the phase ledger
/// (`JUSTRUST_RUSTC_PHASES_FILE`, jr-rustc `jr_phases_write`). Cache the
/// probe against driver metadata, not its installation directory name.
fn phase_ledger_supported(toolchain: &std::path::Path) -> bool {
    use std::hash::{Hash, Hasher};
    let mut drivers: Vec<_> = std::fs::read_dir(toolchain.join("lib"))
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name();
            let n = n.to_string_lossy();
            n.starts_with("librustc_driver") && n.ends_with(".so")
        })
        .filter_map(|e| Some((e.path(), e.metadata().ok()?)))
        .collect();
    drivers.sort_by(|a, b| a.0.cmp(&b.0));
    let mut identity = std::collections::hash_map::DefaultHasher::new();
    for (path, metadata) in &drivers {
        path.hash(&mut identity);
        metadata.len().hash(&mut identity);
        metadata.modified().ok().hash(&mut identity);
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.dev().hash(&mut identity);
            metadata.ino().hash(&mut identity);
            metadata.ctime().hash(&mut identity);
            metadata.ctime_nsec().hash(&mut identity);
        }
    }
    let key = format!("v2:{:016x}", identity.finish());
    let marker = toolchain.join(".jr-phases");
    if let Ok(v) = std::fs::read_to_string(&marker)
        && let Some(answer) = v.strip_prefix(&format!("{key}\n"))
    {
        return answer.trim() == "yes";
    }
    let found = drivers
        .iter()
        .any(|(path, _)| driver_has_phase_ledger(path));
    let tmp = toolchain.join(format!(".jr-phases.{}", std::process::id()));
    if std::fs::write(
        &tmp,
        format!("{key}\n{}\n", if found { "yes" } else { "no" }),
    )
    .is_ok()
    {
        let _ = std::fs::rename(&tmp, &marker);
    }
    found
}

/// Driver libraries exceed 500 MB on some hosted toolchains. Keep the
/// initial probe bounded, including markers split across read boundaries.
fn driver_has_phase_ledger(path: &std::path::Path) -> bool {
    use std::io::Read;
    const MARKER: &[u8] = b"JUSTRUST_RUSTC_PHASES_FILE";
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut buf = [0; 64 * 1024];
    let mut carry = 0;
    loop {
        let n = match file.read(&mut buf[carry..]) {
            Ok(0) => return false,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => return false,
        };
        let end = carry + n;
        if memchr::memmem::find(&buf[..end], MARKER).is_some() {
            return true;
        }
        carry = end.min(MARKER.len() - 1);
        buf.copy_within(end - carry..end, 0);
    }
}

/// Where this unit's compiler writes its phase ledger, when it can.
///
/// On by default for every unit (dependencies too) on a jr compiler that has
/// the ledger: it is an environment variable the compiler reads, not a
/// compiler option, so no incremental, depcache or cargo fingerprint key
/// moves, and it costs no stderr traffic. `JUSTRUST_PASSES=0` turns it off.
fn phase_ledger_path(real: &std::path::Path, run_dir: &std::path::Path) -> Option<PathBuf> {
    if phase_timing_disabled() {
        return None;
    }
    phase_ledger_path_enabled(real, run_dir)
}

fn phase_ledger_path_enabled(real: &std::path::Path, run_dir: &std::path::Path) -> Option<PathBuf> {
    let toolchain = real.parent()?.parent()?;
    if !phase_ledger_supported(toolchain) {
        return None;
    }
    let dir = run_dir.join("phases");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir.join(format!("{}.tsv", std::process::id())))
}

fn phase_timing_disabled() -> bool {
    matches!(
        std::env::var("JUSTRUST_PASSES").as_deref(),
        Ok("0") | Ok("off") | Ok("false") | Ok("no")
    )
}

fn phase_ledger_environment(cmd: &mut Command, ledger: Option<&std::path::Path>) {
    match ledger {
        Some(path) => cmd.env("JUSTRUST_RUSTC_PHASES_FILE", path),
        // Never inherited: a nested build must not write another unit's file.
        None => cmd.env_remove("JUSTRUST_RUSTC_PHASES_FILE"),
    };
}

/// Parse a phase ledger: `label\tseconds\tcount` per line.
pub(crate) fn parse_phase_ledger(text: &str) -> Vec<Pass> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.split('\t');
            let name = f.next().filter(|n| !n.is_empty())?;
            let secs = f.next()?.parse::<f64>().ok().filter(|s| s.is_finite())?;
            Some(Pass {
                name: name.to_owned(),
                secs,
                rss_end_mb: 0.0,
                calls: f.next().and_then(|c| c.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

/// Parallel front end for optimized workspace crates. rustc runs its front
/// end on one thread unless told otherwise, and project wrappers such as
/// Parallelize rustc's front-end for local workspace crates on a jr toolchain
/// (nightly, so `-Z` needs no `RUSTC_BOOTSTRAP` and no cache key moves;
/// `-Zthreads` is UNTRACKED). Only local crates (registry crates come from the
/// depcache), and never when the unit already sets a thread count.
/// `JUSTRUST_FRONTEND_THREADS=0` (or `JUSTRUST_RELEASE_FRONTEND_THREADS=0`)
/// turns it off; `=N` picks the count (default min(16, CPUs), now that
/// jr-rustc parallelizes all CGU MIR-to-LLVM-IR batches under `-Zthreads`).
fn release_frontend_threads_args(real: &std::path::Path, args: &[OsString]) -> Option<OsString> {
    let is_jr = real
        .parent()
        .and_then(|b| b.parent())
        .and_then(|d| d.file_name())
        .is_some_and(|n| n.to_string_lossy().starts_with("jr-"));
    if !is_jr || std::env::var_os("JUSTRUST_TOOLCHAIN_JR").is_none() {
        return None;
    }
    let threads = match std::env::var("JUSTRUST_FRONTEND_THREADS")
        .or_else(|_| std::env::var("JUSTRUST_RELEASE_FRONTEND_THREADS"))
    {
        Ok(v) => v.parse::<usize>().ok()?,
        Err(_) => std::thread::available_parallelism().map_or(1, |n| (n.get() / 2).clamp(1, 16)),
    };
    frontend_threads_for(
        args,
        threads,
        std::env::var_os("CARGO_CRATE_NAME").is_some(),
        std::env::var_os("CARGO_MANIFEST_DIR").map(PathBuf::from),
        std::env::var_os("CARGO_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cargo"))),
    )
}

fn frontend_threads_for(
    args: &[OsString],
    threads: usize,
    cargo_unit: bool,
    manifest: Option<PathBuf>,
    cargo_home: Option<PathBuf>,
) -> Option<OsString> {
    if threads < 2 || !cargo_unit {
        return None;
    }
    let manifest = manifest?;
    if cargo_home.is_some_and(|h| manifest.starts_with(h)) {
        return None;
    }
    let s = |a: &OsString| a.to_string_lossy().into_owned();
    let mut prev = String::new();
    for a in args.iter().map(s) {
        if a.starts_with("-Zthreads")
            || a == "--jobs-frontend"
            || prev == "-Z" && a.starts_with("threads=")
        {
            return None;
        }
        prev = a;
    }
    Some(format!("-Zthreads={threads}").into())
}

/// Cross-crate early cutoff (jr-rustc `-Zjr-iface-hash` + jr-cargo, see
/// `docs/integrations.md`): on a jr toolchain, ask jr-rustc to write the
/// interface hash of every crate (`<rmeta>.jriface`) through its
/// `JUSTRUST_RUSTC_IFACE_HASH` switch, which is not part of any cache key, so
/// cargo fingerprints and rustc's incremental state are the same either way.
/// jr-cargo then skips dependents of a crate whose interface did not change.
/// `JUSTRUST_CUTOFF=0` turns it off (and jr-cargo's half with
/// `JUSTRUST_CARGO_CUTOFF=0`).
fn jr_iface_hash_env(real: &std::path::Path) {
    let is_jr = real
        .parent()
        .and_then(|b| b.parent())
        .and_then(|d| d.file_name())
        .is_some_and(|n| n.to_string_lossy().starts_with("jr-"));
    let off = matches!(
        std::env::var("JUSTRUST_CUTOFF").as_deref(),
        Ok("0") | Ok("off") | Ok("false") | Ok("no")
    );
    if is_jr
        && std::env::var_os("JUSTRUST_TOOLCHAIN_JR").is_some()
        && std::env::var_os("JUSTRUST_RUSTC_IFACE_HASH").is_none()
    {
        // SAFETY: single-threaded here, before any thread or child is spawned.
        unsafe { std::env::set_var("JUSTRUST_RUSTC_IFACE_HASH", if off { "0" } else { "1" }) };
    }
}

fn want_passes(args: &[OsString], local: bool) -> (bool, bool) {
    let has = args
        .iter()
        .any(|a| a.to_string_lossy().starts_with("-Ztime-passes"));
    if has {
        return (false, false);
    }
    match std::env::var("JUSTRUST_PASSES").as_deref() {
        Ok("always") => (true, true),
        Ok("1") | Ok("on") | Ok("true") => (
            std::env::var("RUSTC_BOOTSTRAP").as_deref() == Ok("1")
                // A pinned nightly accepts -Z without RUSTC_BOOTSTRAP, so
                // the cache key does not move. Local crates only: units
                // timed with -Ztime-passes are not stored in the depcache.
                || (local && std::env::var_os("JUSTRUST_TOOLCHAIN_NIGHTLY").is_some()),
            false,
        ),
        _ => (false, false),
    }
}

/// Returns `None` only if the compiler could not be spawned at all.
fn run_unit(
    real: &PathBuf,
    args: &[OsString],
    run_dir: &std::path::Path,
    started: std::time::Instant,
) -> Option<i32> {
    let mut stages = Stages::new(started);
    stages.mark("args");
    let hit_started = std::time::Instant::now();
    let described = describe(args);
    let plan = (!described.local && crate::depcache::enabled())
        .then(|| {
            let env: Vec<(String, String)> = std::env::vars().collect();
            crate::depcache::plan(real, args, &env)
        })
        .flatten();
    stages.mark("key");
    // Local units link with wild when installed (fail open inside the `ld`
    // dispatcher). Never for depcache units (registry crates).
    let nightly = std::env::var_os("JUSTRUST_TOOLCHAIN_NIGHTLY").is_some();
    let mut link_args = match plan {
        None => crate::linker::rustc_args(args, described.local, nightly),
        Some(_) => Vec::new(),
    };
    // Local opt-level=0 codegen units use Cranelift when the active nightly
    // toolchain ships `librustc_codegen_cranelift*.so`. Registry deps stay on
    // LLVM so they hit the prebuilt store; Cranelift links cleanly against
    // LLVM-compiled `.rlib`s (identical `.rmeta` + ELF `.o` ABI). A backend
    // without landing-pad support compiles `panic=abort` units only.
    let cranelift_on = plan.is_none() && cranelift_eligible(real, args, &described, nightly);
    if cranelift_on {
        link_args.push(OsString::from("-Zcodegen-backend=cranelift"));
    }
    // Workspace and path crates: the local cache (`localcache.rs`), keyed
    // with the link arguments the shim adds, since they change the output.
    let plan_t0 = paths::now();
    let local_plan = (described.local && plan.is_none() && crate::localcache::enabled())
        .then(|| {
            let env: Vec<(String, String)> = std::env::vars().collect();
            let salt = link_args
                .iter()
                .map(|a| a.to_string_lossy().into_owned())
                .collect();
            crate::localcache::plan(real, args, &env, salt)
        })
        .flatten();
    let plan_secs = paths::now() - plan_t0;
    stages.mark("local-key");
    if let Some(lp) = &local_plan {
        let start = paths::now();
        if let Some(hit) = crate::localcache::restore(lp, &mut std::io::stderr()) {
            let end = paths::now();
            let mut unit = described;
            stages.apply(&mut unit);
            unit.start = start;
            unit.end = end;
            unit.wall = end - start;
            unit.rmeta_secs = Some(unit.wall);
            unit.depcache = "local".into();
            unit.depcache_saved_secs = Some(hit.saved_secs);
            let _ = crate::buildscript::wrap(&lp.out_dir, &lp.crate_name, &lp.extra);
            unit.cache_hit_overhead_secs = Some(hit_started.elapsed().as_secs_f64());
            write_unit_cost(args, hit.saved_secs, "wall");
            if let Ok(line) = serde_json::to_string(&unit) {
                let _ = paths::append_line(&run_dir.join("units.jsonl"), &line);
            }
            return Some(0);
        }
        stages.mark("miss-localcache");
    }
    if let Some(plan) = &plan {
        let mut err = std::io::stderr();
        // Tiers, cheapest first: this machine, the prebuilt store (static
        // HTTP, anonymous), the account and shared cache (HTTP), then the
        // peer machine over the remote link.
        // A tier that misses is charged to the shim (`miss-<tier>`), and a
        // hit's `wall` is the tier that hit: a miss pays for all four before
        // the compiler starts.
        let mut found = None;
        for (name, missed) in [
            ("hit", "miss-local"),
            ("prebuilt", "miss-prebuilt"),
            ("remote", "miss-remote"),
            ("peer", "miss-peer"),
        ] {
            let start = paths::now();
            let got = match name {
                "hit" => plan.restore(&mut err),
                "prebuilt" => crate::depcache_prebuilt::fetch(plan, &mut err),
                "remote" => crate::depcache_remote::fetch(plan, &mut err),
                _ => crate::peercache::fetch(plan, &mut err),
            };
            if let Some(hit) = got {
                found = Some((name, start, hit));
                break;
            }
            stages.mark(missed);
        }
        if let Some((tier, start, hit)) = found {
            // Downloaded from the account or shared cache: the peer
            // machine likely lacks it too, so offer it on the next sync.
            if matches!(tier, "remote" | "prebuilt")
                && let Some(vv) = crate::depcache::rustc_version(real)
            {
                let tc = crate::peercache::toolchain_tag(&vv);
                crate::peercache::record_store(plan, &hit.line, hit.saved_secs, &tc);
            }
            let end = paths::now();
            let mut unit = described;
            stages.apply(&mut unit);
            unit.start = start;
            unit.end = end;
            unit.wall = end - start;
            unit.rmeta_secs = Some(unit.wall);
            unit.depcache = tier.into();
            unit.depcache_saved_secs = Some(hit.saved_secs);
            let _ = crate::buildscript::wrap(&plan.out_dir, &plan.crate_name, &plan.extra);
            unit.cache_hit_overhead_secs = Some(hit_started.elapsed().as_secs_f64());
            write_unit_cost(args, hit.saved_secs, "wall");
            if let Ok(line) = serde_json::to_string(&unit) {
                let _ = paths::append_line(&run_dir.join("units.jsonl"), &line);
            }
            return Some(0);
        }
    }
    let (passes_on, force_bootstrap) = want_passes(args, described.local);
    let mut cmd = Command::new(real);
    cmd.args(args).stderr(Stdio::piped());
    if let Some(plan) = &plan {
        cmd.args(&plan.extra_args);
    } else if let Some(lp) = &local_plan {
        cmd.args(&lp.extra_args);
    }
    if passes_on {
        cmd.arg("-Ztime-passes").arg("-Ztime-passes-format=json");
    }
    // Explicit -Ztime-passes wins: it has memory figures the ledger lacks.
    let ledger = (!passes_on)
        .then(|| phase_ledger_path(real, run_dir))
        .flatten();
    phase_ledger_environment(&mut cmd, ledger.as_deref());
    if plan.is_none() {
        cmd.args(&link_args);
    }
    if force_bootstrap {
        cmd.env("RUSTC_BOOTSTRAP", "1");
    }
    let early = (plan.is_none() && described.local && described.incremental && early_reply_on())
        .then(|| EarlyReply::attach(&mut cmd))
        .flatten();
    let incr_prior_session = if described.incremental {
        prior_incr_session(args, &described.crate_name)
    } else {
        None
    };
    stages.mark("spawn-setup");
    // Build hosts only: wait until the machine has memory for this unit.
    let memory = crate::memgov::admit(&described, args, run_dir);
    stages.mark(memory.stage());
    let start = paths::now();
    let spawned = cmd.spawn();
    let early = early.map(EarlyReply::spawned);
    let mut child = spawned.ok()?;
    let job = crate::sys::Job::for_child(&child);
    let stderr = child.stderr.take()?;
    let rmeta_at: Arc<Mutex<Option<f64>>> = Arc::default();
    let passes: Arc<Mutex<Vec<Pass>>> = Arc::default();
    let captured: Option<Arc<Mutex<Vec<Vec<u8>>>>> =
        (plan.is_some() || local_plan.is_some() || cranelift_on).then(Arc::default);
    let reader = {
        let rmeta_at = rmeta_at.clone();
        let passes = passes.clone();
        let captured = captured.clone();
        std::thread::spawn(move || {
            forward_stderr(stderr, passes_on, &rmeta_at, &passes, captured.as_deref())
        })
    };
    // Every tier missed: name the key part that moved. This runs beside the
    // compiler, not before it (it cost 0.5 to 0.8s of job time over 206
    // misses when it ran first). The compiler does not rewrite its inputs
    // and the manifest and ledger only change when this shim stores below,
    // so the answer is the same.
    let why = plan
        .as_ref()
        .map(|p| crate::depcache_why::diagnose(p, real, args));

    // jr-rustc replied after the link (outputs complete, no errors, all
    // diagnostics printed) and is still persisting its incremental state:
    // report success now. The next rustc of this crate waits for the
    // persist (jr-rustc `jr_wait_for_persisting`).
    let replied = early.as_ref().is_some_and(EarlyReply::wait);
    let crate::sys::Usage {
        code: exit,
        user,
        sys,
        max_rss_mb,
    } = if replied {
        early_reply_usage(child.id())
    } else {
        crate::sys::wait_usage(&mut child, job.as_ref())
    };
    let end = paths::now();
    stages.resume_after();
    let _ = reader.join();
    if replied {
        // Not reaped: cargo never waits for it, init adopts it on exit.
        std::mem::forget(child);
    }

    // Fail open: if Cranelift hit a backend ICE or unsupported feature, retry
    // the exact same unit with default LLVM so the build still succeeds.
    if cranelift_on
        && exit != 0
        && captured
            .as_ref()
            .is_some_and(|c| cranelift_fallback_needed(&c.lock().unwrap()))
    {
        eprintln!(
            "justrust: cranelift fallback to llvm for {}",
            described.crate_name
        );
        let mut retry = Command::new(real);
        retry.args(args);
        for a in &link_args {
            if a != "-Zcodegen-backend=cranelift" {
                retry.arg(a);
            }
        }
        return retry.status().ok().and_then(|s| s.code());
    }
    memory.release();

    let mut unit = described;
    if let (Some(plan), Some(captured)) = (&plan, &captured) {
        unit.depcache = "miss".into();
        let env: Vec<(String, String)> = std::env::vars().collect();
        unit.keyed_env = crate::depcache::user_keyed_env(&env);
        let (reason, ticket) = why.unwrap_or_default();
        unit.depcache_why = match exit {
            0 if reason.is_empty() => "unexplained: no miss ledger for this toolchain".into(),
            0 => reason,
            _ => "compile-failed".into(),
        };
        if exit == 0 && !passes_on {
            let lines = std::mem::take(&mut *captured.lock().unwrap());
            if let Some(line) = plan.store(start, end - start, &lines) {
                if let Some(t) = &ticket {
                    crate::depcache_why::remember(t);
                }
                crate::depcache_remote::enqueue(plan, &line);
                if crate::depcache::enabled()
                    && let Some(vv) = crate::depcache::rustc_version(real)
                {
                    let tc = crate::peercache::toolchain_tag(&vv);
                    crate::peercache::record_store(plan, &line, end - start, &tc);
                }
            }
            // After storing: the cache keeps the real binary, not the wrapper.
            let _ = crate::buildscript::wrap(&plan.out_dir, &plan.crate_name, &plan.extra);
        }
    }
    if let (Some(lp), Some(captured)) = (&local_plan, &captured) {
        unit.depcache = "local-miss".into();
        if exit == 0 {
            // Pass timings are filtered out of the capture, and
            // -Ztime-passes does not change a local unit's outputs.
            let lines = std::mem::take(&mut *captured.lock().unwrap());
            let t = paths::now();
            let staged = crate::localcache::stage(lp, start, end - start, &lines).is_some();
            // Where the cache's own time goes (FINDINGS 50).
            let line = serde_json::json!({
                "crate": unit.crate_name, "test": unit.test, "plan": plan_secs,
                "stage": paths::now() - t, "staged": staged,
            });
            let _ = paths::append_line(&run_dir.join("localcache.jsonl"), &line.to_string());
            // After staging, as for dependencies: the cache keeps the real
            // binary, and cargo runs the wrapper that replays cached runs
            // of a workspace build script (`buildscript.rs`).
            let _ = crate::buildscript::wrap(&lp.out_dir, &lp.crate_name, &lp.extra);
        }
    }
    unit.start = start;
    unit.end = end;
    unit.wall = end - start;
    unit.user = user;
    unit.sys = sys;
    unit.max_rss_mb = max_rss_mb;
    unit.exit = exit;
    unit.rmeta_secs = rmeta_at.lock().ok().and_then(|g| *g).map(|t| t - start);
    unit.passes = std::mem::take(&mut *passes.lock().unwrap());
    unit.phase_coverage = if passes_on {
        "time-passes"
    } else if ledger.is_some() {
        "native-ledger-missing"
    } else if phase_timing_disabled() {
        "disabled"
    } else {
        // No capable driver, or the recording directory was not writable.
        "native-ledger-unavailable"
    }
    .into();
    if let Some(path) = &ledger {
        // Written when the compiler exits, or just before an early reply
        // (jr-rustc `jr_phases_write_now`): its total is then the time the
        // caller waited, without the background incremental persist.
        if let Ok(text) = std::fs::read_to_string(path) {
            unit.passes = parse_phase_ledger(&text);
            unit.phase_coverage = if unit.passes.is_empty() {
                "native-ledger-empty"
            } else {
                "native-ledger"
            }
            .into();
            let _ = std::fs::remove_file(path);
        }
        let mut reuse_path = path.clone().into_os_string();
        reuse_path.push(".reuse");
        if let Ok(text) = std::fs::read_to_string(&reuse_path) {
            unit.reuse = crate::reuse::parse(&text);
            let _ = std::fs::remove_file(&reuse_path);
        }
    }
    if exit == 0 {
        write_unit_cost(args, user + sys, "cpu");
    }
    crate::memgov::learn(&unit);
    unit.incr_prior_session = incr_prior_session;
    stages.mark("store");
    stages.apply(&mut unit);
    if let Ok(line) = serde_json::to_string(&unit) {
        let _ = paths::append_line(&run_dir.join("units.jsonl"), &line);
    }
    Some(exit)
}

/// Leave what this unit cost beside its outputs
/// (`<out-dir>/.jr-cost-<crate><extra-filename>`: `<secs> <basis>`), so
/// jr-cargo can credit the CPU a later fresh (skipped) build of it avoided.
/// `cpu` is measured compile CPU; on a cache hit the cost stays the compile
/// CPU already recorded there, else the original wall time (`wall`).
fn write_unit_cost(args: &[OsString], secs: f64, basis: &str) {
    let Some(path) = unit_cost_path(args) else {
        return;
    };
    if basis != "cpu"
        && std::fs::read_to_string(&path).is_ok_and(|t| t.trim_end().ends_with(" cpu"))
    {
        return;
    }
    if secs.is_finite() {
        let _ = std::fs::write(&path, format!("{secs:.6} {basis}\n"));
    }
}

fn unit_cost_path(args: &[OsString]) -> Option<PathBuf> {
    let (mut out, mut name, mut extra) = (None, None, String::new());
    let mut it = args.iter().map(|a| a.to_str());
    while let Some(a) = it.next() {
        let a = a?;
        match a {
            "--out-dir" => out = it.next().flatten().map(PathBuf::from),
            "--crate-name" => name = it.next().flatten().map(str::to_owned),
            "-C" => {
                if let Some(e) = it
                    .next()
                    .flatten()
                    .and_then(|v| v.strip_prefix("extra-filename="))
                {
                    extra = e.to_owned();
                }
            }
            _ => {
                if let Some(e) = a.strip_prefix("-Cextra-filename=") {
                    extra = e.to_owned();
                }
            }
        }
    }
    Some(out?.join(format!(".jr-cost-{}{extra}", name?)))
}

/// Let jr-rustc reply as soon as outputs are complete on disk and persist its
/// incremental dep-graph / query cache in the background (default on on Linux;
/// `JUSTRUST_RUSTC_EARLY_REPLY=0` disables). Only rustc builds that know
/// `JUSTRUST_RUSTC_EARLY_REPLY_FD` use the pipe; any other rustc ignores it
/// and is waited for normally.
fn early_reply_on() -> bool {
    cfg!(target_os = "linux")
        && !matches!(
            std::env::var("JUSTRUST_RUSTC_EARLY_REPLY").as_deref(),
            Ok("0") | Ok("off") | Ok("false") | Ok("no")
        )
}

fn early_reply_usage(pid: u32) -> crate::sys::Usage {
    let mut u = crate::sys::Usage::default();
    if let Some(st) = crate::procfs::read_stat(pid as i32) {
        u.user = st.cpu_ticks as f64 / crate::procfs::clk_tck();
        u.max_rss_mb = (st.rss_pages * crate::procfs::page_size()) as f64 / (1024.0 * 1024.0);
    }
    u
}

/// Write end of the early-reply pipe (unix only; never set elsewhere).
#[cfg(unix)]
type ReplyFd = std::os::fd::OwnedFd;
#[cfg(not(unix))]
type ReplyFd = std::fs::File;

/// The pipe jr-rustc writes one byte to once outputs are complete on disk.
struct EarlyReply {
    read: std::fs::File,
    #[allow(dead_code)]
    write: Option<ReplyFd>,
}

impl EarlyReply {
    #[cfg(target_os = "linux")]
    fn attach(cmd: &mut Command) -> Option<EarlyReply> {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let mut fds = [0; 2];
        // SAFETY: pipe2 writes two descriptors into the array on success.
        // Both start close-on-exec; only the write end is then inherited.
        if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
            return None;
        }
        // SAFETY: fresh descriptors owned by nobody else.
        let (read, write) = unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) };
        // SAFETY: clearing FD_CLOEXEC on our own descriptor.
        unsafe { libc::fcntl(write.as_raw_fd(), libc::F_SETFD, 0) };
        cmd.env(
            "JUSTRUST_RUSTC_EARLY_REPLY_FD",
            write.as_raw_fd().to_string(),
        );
        Some(EarlyReply {
            read: std::fs::File::from(read),
            write: Some(write),
        })
    }

    #[cfg(not(target_os = "linux"))]
    fn attach(_cmd: &mut Command) -> Option<EarlyReply> {
        None
    }

    /// Close our copy of the write end once the child has it, so a rustc
    /// that exits without replying shows up as EOF.
    fn spawned(mut self) -> EarlyReply {
        self.write = None;
        self
    }

    /// True if rustc replied; false on EOF (it exited, or never knew the fd).
    fn wait(&self) -> bool {
        use std::io::Read;
        let mut b = [0u8; 1];
        loop {
            match (&self.read).read(&mut b) {
                Ok(1) => return b[0] == 1,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                _ => return false,
            }
        }
    }
}

fn forward_stderr(
    stderr: std::process::ChildStderr,
    passes_on: bool,
    rmeta_at: &Mutex<Option<f64>>,
    passes: &Mutex<Vec<Pass>>,
    captured: Option<&Mutex<Vec<Vec<u8>>>>,
) {
    let mut reader = BufReader::new(stderr);
    let mut out = std::io::stderr();
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
        if passes_on && line.starts_with(b"time: {") {
            if let Some(p) = parse_pass(&line[6..]) {
                // Some passes (drop_ast) are reported once per item, tens of
                // thousands of times. Merge repeats into one entry.
                let mut v = passes.lock().unwrap();
                match v.iter_mut().find(|q| q.name == p.name) {
                    Some(q) => {
                        q.secs += p.secs;
                        q.rss_end_mb = p.rss_end_mb;
                    }
                    None => v.push(p),
                }
            }
            continue;
        }
        if contains(&line, b"\"$message_type\":\"artifact\"")
            && contains(&line, b"\"emit\":\"metadata\"")
        {
            *rmeta_at.lock().unwrap() = Some(paths::now());
        }
        // Forward immediately: cargo relies on artifact notifications for pipelining.
        let _ = out.write_all(&line);
        let _ = out.flush();
        if let Some(c) = captured {
            c.lock().unwrap().push(line.clone());
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

fn parse_pass(json: &[u8]) -> Option<Pass> {
    let v: serde_json::Value = serde_json::from_slice(json).ok()?;
    Some(Pass {
        name: v.get("pass")?.as_str()?.to_owned(),
        secs: v.get("time")?.as_f64()?,
        rss_end_mb: v.get("rss_end").and_then(|r| r.as_f64()).unwrap_or(0.0) / 1_048_576.0,
        calls: 0,
    })
}

fn describe(args: &[OsString]) -> Unit {
    let s: Vec<String> = args
        .iter()
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    let mut u = Unit {
        package: std::env::var("CARGO_PKG_NAME").unwrap_or_default(),
        version: std::env::var("CARGO_PKG_VERSION").unwrap_or_default(),
        manifest_dir: std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default(),
        opt_level: "0".into(),
        ..Default::default()
    };
    let mut i = 0;
    while i < s.len() {
        let a = s[i].as_str();
        let next = s.get(i + 1).map(String::as_str).unwrap_or("");
        let mut codegen = |opt: &str| {
            if let Some(v) = opt.strip_prefix("opt-level=") {
                u.opt_level = v.to_owned();
            } else if opt.starts_with("incremental=") {
                u.incremental = true;
            }
        };
        match a {
            "--crate-name" => {
                u.crate_name = next.to_owned();
                i += 1;
            }
            "--crate-type" => {
                u.crate_types.push(next.to_owned());
                i += 1;
            }
            "--test" => u.test = true,
            "--jobs-frontend" => {
                u.frontend_threads = next.parse().ok();
                i += 1;
            }
            "-Z" => {
                if let Some(v) = next.strip_prefix("threads=") {
                    u.frontend_threads = v.parse().ok();
                }
                i += 1;
            }
            "-O" => u.opt_level = "3".into(),
            "-C" | "--codegen" => {
                codegen(next);
                i += 1;
            }
            _ => {
                if let Some(v) = a.strip_prefix("--emit=") {
                    u.emit = v.to_owned();
                } else if let Some(v) = a
                    .strip_prefix("-C")
                    .or_else(|| a.strip_prefix("--codegen="))
                {
                    codegen(v);
                } else if let Some(v) = a
                    .strip_prefix("-Zthreads=")
                    .or_else(|| a.strip_prefix("--jobs-frontend="))
                {
                    u.frontend_threads = v.parse().ok();
                }
            }
        }
        i += 1;
    }
    u.build_script = u.crate_name.starts_with("build_script_");
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".cargo")))
        .unwrap_or_default();
    u.local = !u.manifest_dir.is_empty()
        && !std::path::Path::new(&u.manifest_dir).starts_with(&cargo_home);
    u
}

fn cranelift_enabled_env() -> bool {
    match std::env::var("JUSTRUST_CRANELIFT") {
        Ok(v) => !matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "off" | "false" | "no" | "llvm"
        ),
        Err(_) => true,
    }
}

/// The Cranelift codegen backend library shipped in the toolchain's sysroot.
fn cranelift_backend(real: &std::path::Path) -> Option<PathBuf> {
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = real;
        None
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        let sysroot = real.parent().and_then(|p| p.parent())?;
        let backends = sysroot
            .join("lib")
            .join("rustlib")
            .join("x86_64-unknown-linux-gnu")
            .join("codegen-backends");
        std::fs::read_dir(backends)
            .ok()?
            .flatten()
            .find(|e| {
                let name = e.file_name();
                let s = name.to_string_lossy();
                s.starts_with("librustc_codegen_cranelift") && s.ends_with(".so")
            })
            .map(|e| e.path())
    }
}

/// Exported by a backend that was built with landing-pad support
/// (`toolchain/rustc/compiler/rustc_codegen_cranelift/src/lib.rs`).
const CRANELIFT_UNWINDING_MARKER: &[u8] = b"JUSTRUST_CG_CLIF_UNWINDING_V1";

/// Can this backend library compile `-Cpanic=unwind` code correctly? One
/// built without its `unwinding` feature emits no landing pads: a
/// `catch_unwind` in the crate does not catch, a panicking thread aborts the
/// whole process (`failed to initiate panic, error 5`) and destructors are
/// skipped while unwinding. That silently changes program behavior, so such
/// a backend is used for `panic=abort` units only.
fn cranelift_backend_unwinds(backend: &std::path::Path) -> bool {
    let probes = crate::paths::home()
        .ok()
        .map(|home| home.join("cache").join("probe"));
    cranelift_backend_unwinds_memo(backend, probes.as_deref())
}

fn cranelift_backend_unwinds_memo(
    backend: &std::path::Path,
    probes: Option<&std::path::Path>,
) -> bool {
    let Ok(meta) = std::fs::metadata(backend) else {
        return false;
    };
    // The library is tens of megabytes: remember the answer per file
    // identity so only the first unit after a toolchain install scans it.
    let memo = probes.map(|dir| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        backend.hash(&mut h);
        meta.len().hash(&mut h);
        meta.modified().ok().hash(&mut h);
        dir.join(format!("clif-unwind-{:016x}", h.finish()))
    });
    if let Some(memo) = &memo {
        match std::fs::read(memo).as_deref() {
            Ok(b"1") => return true,
            Ok(b"0") => return false,
            _ => {}
        }
    }
    let Ok(file) = std::fs::File::open(backend) else {
        return false;
    };
    // SAFETY: a read-only private view of an installed toolchain file.
    let Ok(map) = (unsafe { memmap2::Mmap::map(&file) }) else {
        return false;
    };
    let unwinds = memchr::memmem::find(&map, CRANELIFT_UNWINDING_MARKER).is_some();
    if let Some(memo) = &memo
        && let Some(dir) = memo.parent()
        && std::fs::create_dir_all(dir).is_ok()
    {
        let _ = std::fs::write(memo, if unwinds { b"1" } else { b"0" });
    }
    unwinds
}

/// Does the unit compile with `-C panic=abort`? (The last `-C panic` wins.)
fn panic_aborts(args: &[OsString]) -> bool {
    let mut strategy = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].to_string_lossy();
        let cg = if a == "-C" || a == "--codegen" {
            i += 1;
            args.get(i).map(|x| x.to_string_lossy().into_owned())
        } else {
            a.strip_prefix("-C")
                .or_else(|| a.strip_prefix("--codegen="))
                .map(str::to_owned)
        };
        if let Some(v) = cg.as_deref().and_then(|c| c.strip_prefix("panic=")) {
            strategy = Some(v.to_owned());
        }
        i += 1;
    }
    matches!(strategy.as_deref(), Some("abort" | "immediate-abort"))
}

fn cranelift_args_eligible(args: &[OsString], described: &Unit) -> bool {
    if !described.local || described.opt_level != "0" || described.build_script {
        return false;
    }
    if described
        .crate_types
        .iter()
        .any(|t| matches!(t.as_str(), "proc-macro" | "cdylib" | "staticlib"))
    {
        return false;
    }
    let emits_codegen = described.emit.is_empty()
        || described
            .emit
            .split(',')
            .any(|e| matches!(e, "link" | "obj"));
    if !emits_codegen {
        return false;
    }
    let mut i = 0;
    while i < args.len() {
        let a = args[i].to_string_lossy();
        let next = args
            .get(i + 1)
            .map(|x| x.to_string_lossy())
            .unwrap_or_default();
        if a == "--target" || a.starts_with("--target=") {
            return false;
        }
        if a == "-Z" {
            if next.starts_with("codegen-backend") || next.starts_with("sanitizer") {
                return false;
            }
            i += 2;
            continue;
        }
        if a.starts_with("-Zcodegen-backend") || a.starts_with("-Zsanitizer") {
            return false;
        }
        let cg = if a == "-C" || a == "--codegen" {
            i += 1;
            Some(next.as_ref())
        } else {
            a.strip_prefix("-C")
                .or_else(|| a.strip_prefix("--codegen="))
        };
        if let Some(opt) = cg
            && (opt.starts_with("instrument-coverage")
                || opt.starts_with("linker-plugin-lto")
                || opt == "embed-bitcode=yes"
                || opt == "lto"
                || opt
                    .strip_prefix("lto=")
                    .is_some_and(|v| !matches!(v, "off" | "false" | "no" | "n")))
        {
            return false;
        }
        i += 1;
    }
    true
}

fn cranelift_eligible(
    real: &std::path::Path,
    args: &[OsString],
    described: &Unit,
    nightly: bool,
) -> bool {
    nightly
        && cranelift_enabled_env()
        && cranelift_args_eligible(args, described)
        && cranelift_backend(real)
            .is_some_and(|backend| panic_aborts(args) || cranelift_backend_unwinds(&backend))
}

fn cranelift_fallback_needed(captured: &[Vec<u8>]) -> bool {
    captured.iter().any(|line| {
        contains(line, b"rustc_codegen_cranelift")
            || contains(line, b"cg_clif")
            || contains(line, b"internal compiler error")
            || contains(line, b"cranelift")
    })
}

/// Shim entry check: was this process started as `rustc` by justrust?
pub fn invoked_as_shim(argv0: &OsStr) -> bool {
    crate::sys::program_name(argv0) == "rustc"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_cost_sits_beside_outputs() {
        let a = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        assert_eq!(
            unit_cost_path(&a(&[
                "--crate-name",
                "foo",
                "--out-dir",
                "/t/deps",
                "-C",
                "extra-filename=-ab12"
            ])),
            Some(PathBuf::from("/t/deps/.jr-cost-foo-ab12"))
        );
        assert_eq!(
            unit_cost_path(&a(&[
                "--crate-name",
                "foo",
                "-Cextra-filename=-cd",
                "--out-dir",
                "/o"
            ])),
            Some(PathBuf::from("/o/.jr-cost-foo-cd"))
        );
        assert_eq!(unit_cost_path(&a(&["--crate-name", "foo"])), None);
        let d = std::env::temp_dir().join(format!("jr-unit-cost-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let args = a(&["--crate-name", "x", "--out-dir", d.to_str().unwrap()]);
        write_unit_cost(&args, 2.5, "cpu");
        write_unit_cost(&args, 9.0, "wall");
        let t = std::fs::read_to_string(d.join(".jr-cost-x")).unwrap();
        let _ = std::fs::remove_dir_all(&d);
        assert_eq!(t, "2.500000 cpu\n", "a hit keeps the measured CPU");
    }

    #[test]
    fn slot_incremental_maps_to_shared_dir() {
        // The base is passed in: a hosted test run inherits the agent's
        // `JUSTRUST_SHARED_INCREMENTAL_DIR`.
        let m = |p: &str| shared_incremental_dir(std::path::Path::new(p), None);
        assert_eq!(
            m("/w/target/justrust-slots/2/debug/incremental"),
            Some(PathBuf::from("/w/target/debug/incremental"))
        );
        assert_eq!(
            m("/w/target/justrust-slots/0/x86_64-unknown-linux-gnu/release/incremental"),
            Some(PathBuf::from(
                "/w/target/x86_64-unknown-linux-gnu/release/incremental"
            ))
        );
        assert_eq!(m("/w/target/debug/incremental"), None);
        assert_eq!(m("/w/target/justrust-slots/a/debug/incremental"), None);
        assert_eq!(m("/w/target/justrust-slots/1/debug/deps"), None);
        let mut a: Vec<OsString> = [
            "-C",
            "incremental=/t/justrust-slots/3/debug/incremental",
            "-C",
            "opt-level=0",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        share_slot_incremental_in(&mut a, None);
        let expected: PathBuf = PathBuf::from("/t/debug/incremental").components().collect();
        assert_eq!(
            a[1],
            OsString::from(format!("incremental={}", expected.display()))
        );
        let mut b = vec![OsString::from(
            "-Cincremental=/t/justrust-slots/1/debug/incremental",
        )];
        share_slot_incremental_in(&mut b, None);
        assert_eq!(
            b[0],
            OsString::from(format!("-Cincremental={}", expected.display()))
        );
        // A hosted base wins for any worktree and keeps only the profile.
        let base = std::env::temp_dir().join(format!("jr-shared-incr-{}", std::process::id()));
        let mut c = vec![OsString::from(
            "-Cincremental=/t/justrust-slots/1/debug/incremental",
        )];
        share_slot_incremental_in(&mut c, Some(base.as_os_str()));
        let shared = base.join("debug").join("incremental");
        assert_eq!(
            c[0],
            OsString::from(format!("-Cincremental={}", shared.display()))
        );
        assert!(shared.is_dir());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn frontend_threads_for_all_local_units() {
        let os = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        let home = Some(PathBuf::from("/h/.cargo"));
        let local = Some(PathBuf::from("/w/crates/ui"));
        let reg = Some(PathBuf::from("/h/.cargo/registry/src/x/serde-1"));
        let rel = os(&["--crate-name", "ui", "-C", "opt-level=3"]);
        let f = |a: &[OsString], m: &Option<PathBuf>, n| {
            frontend_threads_for(a, n, true, m.clone(), home.clone())
        };
        assert_eq!(f(&rel, &local, 8), Some("-Zthreads=8".into()));
        assert_eq!(
            f(&os(&["-Copt-level=s"]), &local, 4),
            Some("-Zthreads=4".into())
        );
        assert_eq!(f(&rel, &reg, 8), None, "registry crate");
        assert_eq!(f(&rel, &local, 1), None, "disabled");
        assert_eq!(
            f(&os(&["-C", "opt-level=0"]), &local, 8),
            Some("-Zthreads=8".into()),
            "debug"
        );
        assert_eq!(
            f(&os(&["--crate-name", "ui"]), &local, 8),
            Some("-Zthreads=8".into()),
            "no opt-level"
        );
        assert_eq!(
            f(&os(&["-C", "opt-level=3", "-Zthreads=2"]), &local, 8),
            None
        );
        assert_eq!(
            f(&os(&["-C", "opt-level=3", "-Z", "threads=2"]), &local, 8),
            None
        );
        assert_eq!(
            frontend_threads_for(&rel, 8, false, local.clone(), home.clone()),
            None
        );
    }

    #[test]
    fn proc_macro_input_enforcement_required_from_driver() {
        let dir = std::env::temp_dir().join(format!("jr-pmi-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        // Neither an old positive marker nor the observational query is enough.
        std::fs::write(dir.join(".jr-pm-inputs"), "yes\n").unwrap();
        std::fs::write(
            dir.join("lib/librustc_driver-x.so"),
            b"..jr_proc_macro_inputs..",
        )
        .unwrap();
        assert!(!proc_macro_inputs_enforced(&dir));
        assert_eq!(
            std::fs::read_to_string(dir.join(".jr-pm-inputs-enforced-v1")).unwrap(),
            "no\n"
        );
        std::fs::remove_file(dir.join(".jr-pm-inputs-enforced-v1")).unwrap();
        std::fs::write(
            dir.join("lib/librustc_driver-x.so"),
            b"..jr_proc_macro_inputs_enforced_v1..",
        )
        .unwrap();
        assert!(proc_macro_inputs_enforced(&dir));
        assert!(proc_macro_inputs_enforced(&dir), "cached");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cache_proc_macros_only_for_incremental_units_on_jr() {
        let os = |v: &[&str]| v.iter().map(OsString::from).collect::<Vec<_>>();
        let jr = std::path::Path::new("/h/.justrust/toolchains/jr-ac20de5e7921/bin/rustc");
        let other = std::path::Path::new("/usr/bin/rustc");
        let incr = os(&["--crate-name", "ui", "-C", "incremental=/t/incremental"]);
        let incr2 = os(&["--crate-name", "ui", "-Cincremental=/t/incremental"]);
        let dep = os(&["--crate-name", "serde", "-C", "opt-level=2"]);
        let has = os(&["-Zcache-proc-macros", "-C", "incremental=/t"]);
        // SAFETY: tests in this module that touch these variables run in
        // this one function, single-threaded.
        unsafe {
            std::env::remove_var("JUSTRUST_TOOLCHAIN_JR");
            std::env::remove_var("JUSTRUST_CACHE_PROC_MACROS");
        }
        assert_eq!(cache_proc_macro_args(jr, &incr), None, "not jr: never");
        unsafe { std::env::set_var("JUSTRUST_TOOLCHAIN_JR", "1") };
        assert_eq!(
            cache_proc_macro_args(jr, &incr),
            None,
            "untracked (missing) toolchain stays off by default"
        );
        for value in ["0", "off", "false", "no"] {
            unsafe { std::env::set_var("JUSTRUST_CACHE_PROC_MACROS", value) };
            assert_eq!(
                cache_proc_macro_args(jr, &incr),
                None,
                "not explicit opt-in: {value}"
            );
        }
        for value in ["1", "on", "true", "yes"] {
            unsafe { std::env::set_var("JUSTRUST_CACHE_PROC_MACROS", value) };
            assert_eq!(
                cache_proc_macro_args(jr, &incr).as_deref(),
                Some("-Zcache-proc-macros".as_ref()),
                "explicit experiment: {value}",
            );
        }
        assert_eq!(
            cache_proc_macro_args(other, &incr),
            None,
            "inherited var, other rustc"
        );
        assert!(cache_proc_macro_args(jr, &incr2).is_some());
        assert_eq!(
            cache_proc_macro_args(jr, &dep),
            None,
            "non-incremental deps keep their key"
        );
        assert_eq!(cache_proc_macro_args(jr, &has), None, "no duplicate");
        unsafe { std::env::set_var("JUSTRUST_CACHE_PROC_MACROS", "0") };
        assert_eq!(cache_proc_macro_args(jr, &incr), None, "opt-out");
        unsafe {
            std::env::remove_var("JUSTRUST_TOOLCHAIN_JR");
            std::env::remove_var("JUSTRUST_CACHE_PROC_MACROS");
        }
    }

    #[test]
    fn describes_cargo_rustc_args() {
        let args: Vec<OsString> = [
            "--crate-name",
            "jcode_desktop_ui",
            "--edition=2024",
            "src/lib.rs",
            "--emit=dep-info,metadata,link",
            "-C",
            "opt-level=0",
            "--test",
            "-C",
            "incremental=/t/incremental",
            "-Zthreads=8",
        ]
        .iter()
        .map(OsString::from)
        .collect();
        let u = describe(&args);
        assert_eq!(u.crate_name, "jcode_desktop_ui");
        assert!(u.test && u.incremental);
        assert_eq!(u.frontend_threads, Some(8));
        assert_eq!(u.emit, "dep-info,metadata,link");
    }

    #[test]
    fn describes_effective_opt_level() {
        for (flags, expected) in [
            (vec!["-Copt-level=s", "-O"], "3"),
            (vec!["-O", "--codegen=opt-level=z"], "z"),
            (vec!["--codegen", "opt-level=1"], "1"),
        ] {
            let args: Vec<_> = flags.iter().map(OsString::from).collect();
            assert_eq!(describe(&args).opt_level, expected);
        }
    }

    #[test]
    fn describes_frontend_thread_option_spellings() {
        for flags in [
            vec!["-Zthreads=8"],
            vec!["-Z", "threads=8"],
            vec!["--jobs-frontend=8"],
            vec!["--jobs-frontend", "8"],
        ] {
            let args = flags.iter().map(OsString::from).collect::<Vec<_>>();
            assert_eq!(describe(&args).frontend_threads, Some(8), "{flags:?}");
        }
        for flags in [
            vec!["--jobs", "8"],
            vec!["--jobs-backend=8"],
            vec!["-Z", "time-passes"],
        ] {
            let args = flags.iter().map(OsString::from).collect::<Vec<_>>();
            assert_eq!(describe(&args).frontend_threads, None, "{flags:?}");
        }
    }

    #[test]
    fn parses_phase_ledger() {
        let p = parse_phase_ledger(
            "type_check_crate\t2.089000\t1\nexpand_proc_macro\t0.400000\t3120\n\nbad\tx\t1\ntotal\t6.976000\t1\n",
        );
        assert_eq!(p.len(), 3);
        assert_eq!(p[0].name, "type_check_crate");
        assert_eq!(p[0].secs, 2.089);
        assert_eq!(
            (p[1].name.as_str(), p[1].calls),
            ("expand_proc_macro", 3120)
        );
        assert_eq!(p[2].name, "total");
    }

    #[test]
    fn native_phase_setup_preserves_arguments_and_cache_environment() {
        let args = [
            "--crate-name",
            "example",
            "-Cincremental=/tmp/inc",
            "--emit=metadata",
        ];
        let mut cmd = Command::new("rustc");
        cmd.args(args);
        phase_ledger_environment(&mut cmd, Some(std::path::Path::new("/record/phases.tsv")));
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), args.map(OsStr::new));
        let env = cmd.get_envs().collect::<Vec<_>>();
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].0, "JUSTRUST_RUSTC_PHASES_FILE");
        assert_eq!(env[0].1, Some(OsStr::new("/record/phases.tsv")));
        assert!(!crate::depcache::keyed_env("JUSTRUST_RUSTC_PHASES_FILE"));
        phase_ledger_environment(&mut cmd, None);
        assert_eq!(cmd.get_args().collect::<Vec<_>>(), args.map(OsStr::new));
        assert_eq!(
            cmd.get_envs().collect::<Vec<_>>(),
            [(OsStr::new("JUSTRUST_RUSTC_PHASES_FILE"), None)]
        );
    }

    #[test]
    fn phase_coverage_is_backward_compatible() {
        let mut value = serde_json::to_value(Unit::default()).unwrap();
        assert!(value.get("phase_coverage").is_none());
        let old: Unit = serde_json::from_value(value.clone()).unwrap();
        assert!(old.phase_coverage.is_empty());
        value["phase_coverage"] = "native-ledger-unavailable".into();
        let new: Unit = serde_json::from_value(value).unwrap();
        assert_eq!(new.phase_coverage, "native-ledger-unavailable");
    }

    #[test]
    fn phase_ledger_probe_handles_chunk_boundaries() {
        let path = std::env::temp_dir().join(format!("jr-phase-boundary-{}", std::process::id()));
        let marker = b"JUSTRUST_RUSTC_PHASES_FILE";
        for split in 0..marker.len() {
            let mut data = vec![b'.'; 64 * 1024 - split];
            data.extend_from_slice(marker);
            std::fs::write(&path, data).unwrap();
            assert!(driver_has_phase_ledger(&path), "split {split}");
        }
        std::fs::write(&path, b"JUSTRUST_RUSTC_PHASES_FIL").unwrap();
        assert!(!driver_has_phase_ledger(&path));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn phase_ledger_detected_from_driver() {
        let dir = std::env::temp_dir().join(format!("hosted-compiler-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("lib")).unwrap();
        std::fs::write(dir.join("lib/librustc_driver-x.so"), b"old driver").unwrap();
        assert!(!phase_ledger_supported(&dir));
        assert!(phase_ledger_path_enabled(&dir.join("bin/rustc"), &dir.join("record")).is_none());
        assert!(!dir.join("record").exists());
        // Legacy negative probes must not hide a replacement driver.
        std::fs::write(dir.join(".jr-phases"), "no\n").unwrap();
        std::fs::write(
            dir.join("lib/librustc_driver-x.so"),
            b"..JUSTRUST_RUSTC_PHASES_FILE..",
        )
        .unwrap();
        assert!(phase_ledger_supported(&dir));
        assert!(phase_ledger_supported(&dir), "cached");
        let ledger = phase_ledger_path_enabled(&dir.join("bin/rustc"), &dir.join("record"));
        assert_eq!(
            ledger,
            Some(dir.join(format!("record/phases/{}.tsv", std::process::id())))
        );
        #[cfg(unix)]
        {
            let driver = dir.join("lib/librustc_driver-x.so");
            let metadata = std::fs::metadata(&driver).unwrap();
            let replacement = dir.join("lib/replacement");
            std::fs::write(&replacement, vec![b'.'; metadata.len() as usize]).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&replacement)
                .unwrap()
                .set_times(std::fs::FileTimes::new().set_modified(metadata.modified().unwrap()))
                .unwrap();
            std::fs::rename(replacement, driver).unwrap();
            assert!(!phase_ledger_supported(&dir), "inode replacement");
        }
        std::fs::write(dir.join("lib/librustc_driver-x.so"), b"replacement").unwrap();
        assert!(!phase_ledger_supported(&dir));
        std::fs::write(
            dir.join("lib/librustc_driver-x.so"),
            b"new JUSTRUST_RUSTC_PHASES_FILE driver",
        )
        .unwrap();
        assert!(phase_ledger_supported(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn parses_time_passes_json() {
        let p = parse_pass(
            br#"{"pass":"type_check_crate","time":1.5,"rss_start":1,"rss_end":2097152}"#,
        )
        .unwrap();
        assert_eq!(p.name, "type_check_crate");
        assert_eq!(p.secs, 1.5);
        assert_eq!(p.rss_end_mb, 2.0);
    }

    #[test]
    fn cranelift_eligibility_filters_check_release_cross_and_user_backend() {
        let os = |s: &[&str]| s.iter().map(OsString::from).collect::<Vec<_>>();
        let mut u = Unit {
            local: true,
            opt_level: "0".into(),
            emit: "dep-info,link".into(),
            ..Default::default()
        };
        assert!(cranelift_args_eligible(&os(&["--crate-name", "foo"]), &u));
        // cargo check (metadata only) -> skip
        u.emit = "dep-info,metadata".into();
        assert!(!cranelift_args_eligible(&os(&["--crate-name", "foo"]), &u));
        u.emit = "dep-info,link".into();
        // release / optimized -> skip
        u.opt_level = "3".into();
        assert!(!cranelift_args_eligible(&os(&["--crate-name", "foo"]), &u));
        u.opt_level = "0".into();
        // cross / musl target -> skip
        assert!(!cranelift_args_eligible(
            &os(&["--target", "x86_64-unknown-linux-musl"]),
            &u
        ));
        // user-specified backend -> skip
        assert!(!cranelift_args_eligible(
            &os(&["-Zcodegen-backend=llvm"]),
            &u
        ));
        // build script & proc-macro -> skip
        u.build_script = true;
        assert!(!cranelift_args_eligible(&os(&[]), &u));
        u.build_script = false;
        u.crate_types = vec!["proc-macro".into()];
        assert!(!cranelift_args_eligible(&os(&[]), &u));
        u.crate_types.clear();
        // non-local registry dep -> skip
        u.local = false;
        assert!(!cranelift_args_eligible(&os(&[]), &u));
    }

    #[test]
    fn cranelift_needs_an_unwinding_backend_unless_the_unit_aborts() {
        let os = |s: &[&str]| s.iter().map(OsString::from).collect::<Vec<_>>();
        assert!(!panic_aborts(&os(&["--crate-name", "foo", "--test"])));
        assert!(!panic_aborts(&os(&["-C", "panic=unwind"])));
        assert!(panic_aborts(&os(&["-C", "panic=abort"])));
        assert!(panic_aborts(&os(&["-Cpanic=abort", "-C", "opt-level=0"])));
        assert!(panic_aborts(&os(&["--codegen=panic=immediate-abort"])));
        // The last strategy wins, as in rustc.
        assert!(!panic_aborts(&os(&["-Cpanic=abort", "-C", "panic=unwind"])));

        let dir = std::env::temp_dir().join(format!("jr-clif-unwind-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let probes = dir.join("probe");
        let plain = dir.join("librustc_codegen_cranelift-old.so");
        std::fs::write(&plain, b"\x7fELF __rustc_codegen_backend").unwrap();
        let unwinding = dir.join("librustc_codegen_cranelift-new.so");
        let mut bytes = b"\x7fELF __rustc_codegen_backend ".to_vec();
        bytes.extend_from_slice(CRANELIFT_UNWINDING_MARKER);
        std::fs::write(&unwinding, &bytes).unwrap();
        for _memoized in 0..2 {
            assert!(!cranelift_backend_unwinds_memo(&plain, Some(&probes)));
            assert!(cranelift_backend_unwinds_memo(&unwinding, Some(&probes)));
        }
        assert_eq!(std::fs::read_dir(&probes).unwrap().count(), 2);
        assert!(cranelift_backend_unwinds_memo(&unwinding, None));
        assert!(!cranelift_backend_unwinds_memo(
            &dir.join("missing.so"),
            Some(&probes)
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cranelift_fallback_matches_backend_errors_not_normal_compile_errors() {
        assert!(cranelift_fallback_needed(&[
            b"error: internal compiler error: rustc_codegen_cranelift panicked".to_vec()
        ]));
        assert!(!cranelift_fallback_needed(&[
            b"error[E0308]: mismatched types".to_vec()
        ]));
    }
}
