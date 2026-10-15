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
    /// Shared dependency cache outcome: "hit", "miss" (compiled, stored when
    /// it succeeded), or empty when the unit was not eligible (local,
    /// incremental, disabled with `JUSTRUST_DEPCACHE=0`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub depcache: String,
    /// On a hit, how long the original compile took.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depcache_saved_secs: Option<f64>,
    /// On a miss: keyed env vars cargo does not set itself (user env or a
    /// cargo `[env]`), which keep this unit's key from matching the store.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyed_env: Vec<String>,
}

#[derive(Debug, Serialize, serde::Deserialize, Clone)]
pub struct Pass {
    pub name: String,
    pub secs: f64,
    pub rss_end_mb: f64,
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
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let real = real_rustc();
    args.extend(cache_proc_macro_args(&real, &args));
    jr_iface_hash_env(&real);
    let run_dir = std::env::var_os("JUSTRUST_RUN_DIR").map(PathBuf::from);
    let is_unit =
        std::env::var_os("CARGO_CRATE_NAME").is_some() && args.iter().any(|a| a == "--crate-name");
    let Some(run_dir) = run_dir.filter(|_| is_unit) else {
        exec(&real, &args);
    };
    match run_unit(&real, &args, &run_dir) {
        Some(code) => std::process::exit(code),
        None => exec(&real, &args),
    }
}

fn exec(real: &PathBuf, args: &[OsString]) -> ! {
    let err = crate::sys::exec(Command::new(real).args(args));
    eprintln!("justrust: failed to exec {}: {err}", real.display());
    std::process::exit(127);
}

/// `-Zcache-proc-macros` for local incremental units when the build runs
/// justrust's own compiler (`JUSTRUST_TOOLCHAIN_JR`): jr-rustc then reuses
/// derive and attribute proc-macro output between edits (FINDINGS 29,
/// reference loop -23%). Same answer for every unit of a crate in every
/// run, because the flag is part of rustc's incremental key: it depends only
/// on the environment and the unit's own arguments.
/// `JUSTRUST_CACHE_PROC_MACROS=0` turns it off.
fn cache_proc_macro_args(real: &std::path::Path, args: &[OsString]) -> Option<OsString> {
    // The variable is inherited by nested cargo invocations (build scripts,
    // xtasks) that may run another rustc: also require that the compiler
    // really is a jr toolchain's (`.../toolchains/jr-*/bin/rustc`).
    let is_jr = real
        .parent()
        .and_then(|b| b.parent())
        .and_then(|d| d.file_name())
        .is_some_and(|n| n.to_string_lossy().starts_with("jr-"));
    if !is_jr
        || std::env::var_os("JUSTRUST_TOOLCHAIN_JR").is_none()
        || matches!(
            std::env::var("JUSTRUST_CACHE_PROC_MACROS").as_deref(),
            Ok("0") | Ok("off") | Ok("false") | Ok("no")
        )
    {
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
fn run_unit(real: &PathBuf, args: &[OsString], run_dir: &std::path::Path) -> Option<i32> {
    let described = describe(args);
    let plan = (!described.local && crate::depcache::enabled())
        .then(|| {
            let env: Vec<(String, String)> = std::env::vars().collect();
            crate::depcache::plan(real, args, &env)
        })
        .flatten();
    // Local units link with wild when installed (fail open inside the `ld`
    // dispatcher). Never for depcache units (registry crates).
    let nightly = std::env::var_os("JUSTRUST_TOOLCHAIN_NIGHTLY").is_some();
    let link_args = match plan {
        None => crate::linker::rustc_args(args, described.local, nightly),
        Some(_) => Vec::new(),
    };
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
    if let Some(lp) = &local_plan {
        let start = paths::now();
        if let Some(hit) = crate::localcache::restore(lp, &mut std::io::stderr()) {
            let end = paths::now();
            let mut unit = described;
            unit.start = start;
            unit.end = end;
            unit.wall = end - start;
            unit.rmeta_secs = Some(unit.wall);
            unit.depcache = "local".into();
            unit.depcache_saved_secs = Some(hit.saved_secs);
            if let Ok(line) = serde_json::to_string(&unit) {
                let _ = paths::append_line(&run_dir.join("units.jsonl"), &line);
            }
            return Some(0);
        }
    }
    if let Some(plan) = &plan {
        let start = paths::now();
        let mut err = std::io::stderr();
        // Tiers, cheapest first: this machine, the prebuilt store (static
        // HTTP, anonymous), the account and shared cache (HTTP), then the
        // peer machine over the remote link.
        let mut tier = "hit";
        let found = plan
            .restore(&mut err)
            .or_else(|| {
                tier = "prebuilt";
                crate::depcache_prebuilt::fetch(plan, &mut err)
            })
            .or_else(|| {
                tier = "remote";
                crate::depcache_remote::fetch(plan, &mut err)
            })
            .or_else(|| {
                tier = "peer";
                crate::peercache::fetch(plan, &mut err)
            });
        if let Some(hit) = found {
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
            unit.start = start;
            unit.end = end;
            unit.wall = end - start;
            unit.rmeta_secs = Some(unit.wall);
            unit.depcache = tier.into();
            unit.depcache_saved_secs = Some(hit.saved_secs);
            let _ = crate::buildscript::wrap(&plan.out_dir, &plan.crate_name, &plan.extra);
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
    }
    if passes_on {
        cmd.arg("-Ztime-passes").arg("-Ztime-passes-format=json");
    }
    if plan.is_none() {
        let hot =
            crate::hotpatch::rustc_args(described.local, described.test, !link_args.is_empty());
        cmd.args(&link_args);
        if !hot.is_empty() {
            cmd.args(hot);
            let (k, v) = crate::hotpatch::link_env();
            cmd.env(k, v);
        }
    }
    if force_bootstrap {
        cmd.env("RUSTC_BOOTSTRAP", "1");
    }
    let early = (plan.is_none() && described.local && described.incremental && early_reply_on())
        .then(|| EarlyReply::attach(&mut cmd))
        .flatten();
    let start = paths::now();
    let spawned = cmd.spawn();
    let early = early.map(EarlyReply::spawned);
    let mut child = spawned.ok()?;
    let job = crate::sys::Job::for_child(&child);
    let stderr = child.stderr.take()?;
    let rmeta_at: Arc<Mutex<Option<f64>>> = Arc::default();
    let passes: Arc<Mutex<Vec<Pass>>> = Arc::default();
    let captured: Option<Arc<Mutex<Vec<Vec<u8>>>>> =
        (plan.is_some() || local_plan.is_some()).then(Arc::default);
    let reader = {
        let rmeta_at = rmeta_at.clone();
        let passes = passes.clone();
        let captured = captured.clone();
        std::thread::spawn(move || {
            forward_stderr(stderr, passes_on, &rmeta_at, &passes, captured.as_deref())
        })
    };

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
        crate::sys::Usage::default()
    } else {
        crate::sys::wait_usage(&mut child, job.as_ref())
    };
    let end = paths::now();
    let _ = reader.join();
    if replied {
        // Not reaped: cargo never waits for it, init adopts it on exit.
        std::mem::forget(child);
    }

    let mut unit = described;
    if let (Some(plan), Some(captured)) = (&plan, &captured) {
        unit.depcache = "miss".into();
        let env: Vec<(String, String)> = std::env::vars().collect();
        unit.keyed_env = crate::depcache::user_keyed_env(&env);
        if exit == 0 && !passes_on {
            let lines = std::mem::take(&mut *captured.lock().unwrap());
            if let Some(line) = plan.store(start, end - start, &lines) {
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
    if let Ok(line) = serde_json::to_string(&unit) {
        let _ = paths::append_line(&run_dir.join("units.jsonl"), &line);
    }
    Some(exit)
}

/// `JUSTRUST_RUSTC_EARLY_REPLY=1`: let jr-rustc reply after the link
/// and persist its incremental state after the shim returned (opt-in,
/// FINDINGS 60). Only rustc builds that know `JUSTRUST_RUSTC_EARLY_REPLY_FD`
/// use the pipe; any other rustc ignores it and is waited for as usual.
fn early_reply_on() -> bool {
    cfg!(target_os = "linux")
        && matches!(
            std::env::var("JUSTRUST_RUSTC_EARLY_REPLY").as_deref(),
            Ok("1") | Ok("on") | Ok("true")
        )
}

/// Write end of the early-reply pipe (unix only; never set elsewhere).
#[cfg(unix)]
type ReplyFd = std::os::fd::OwnedFd;
#[cfg(not(unix))]
type ReplyFd = std::fs::File;

/// The pipe jr-rustc writes one byte to once it can be considered done.
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
            "-C" => {
                codegen(next);
                i += 1;
            }
            _ => {
                if let Some(v) = a.strip_prefix("--emit=") {
                    u.emit = v.to_owned();
                } else if let Some(v) = a.strip_prefix("-C") {
                    codegen(v);
                } else if let Some(v) = a.strip_prefix("-Zthreads=") {
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

/// Shim entry check: was this process started as `rustc` by justrust?
pub fn invoked_as_shim(argv0: &OsStr) -> bool {
    crate::sys::program_name(argv0) == "rustc"
}

#[cfg(test)]
mod tests {
    use super::*;

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
            cache_proc_macro_args(jr, &incr).as_deref(),
            Some("-Zcache-proc-macros".as_ref())
        );
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
    fn parses_time_passes_json() {
        let p = parse_pass(
            br#"{"pass":"type_check_crate","time":1.5,"rss_start":1,"rss_end":2097152}"#,
        )
        .unwrap();
        assert_eq!(p.name, "type_check_crate");
        assert_eq!(p.secs, 1.5);
        assert_eq!(p.rss_end_mb, 2.0);
    }
}
