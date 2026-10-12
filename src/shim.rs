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
use std::os::unix::process::CommandExt;
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
    let err = Command::new(real).args(args).exec();
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
        if let Some(hit) = lp.restore(&mut std::io::stderr()) {
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
    let start = paths::now();
    let mut child = cmd.spawn().ok()?;
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

    let (exit, user, sys, max_rss_mb) = wait4(child.id() as i32);
    let end = paths::now();
    let _ = reader.join();

    let mut unit = described;
    if let (Some(plan), Some(captured)) = (&plan, &captured) {
        unit.depcache = "miss".into();
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
            // Where the cache's own time goes (FINDINGS 46).
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

/// Wait for `pid`, returning (exit code, user secs, sys secs, max RSS MiB).
/// Linux reports the child's own usage plus its waited-for descendants
/// (linker, `cc`), so link time is included.
fn wait4(pid: i32) -> (i32, f64, f64, f64) {
    let mut status = 0;
    // SAFETY: rusage is plain data and fully written by wait4 on success.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: valid pointers to local storage.
        let r = unsafe { libc::wait4(pid, &mut status, 0, &mut ru) };
        if r == pid {
            break;
        }
        if r == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return (1, 0.0, 0.0, 0.0);
    }
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else if libc::WIFSIGNALED(status) {
        128 + libc::WTERMSIG(status)
    } else {
        1
    };
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    // ru_maxrss is KiB on Linux and bytes on macOS.
    let rss_mb = if cfg!(target_os = "macos") {
        ru.ru_maxrss as f64 / 1_048_576.0
    } else {
        ru.ru_maxrss as f64 / 1024.0
    };
    (code, tv(ru.ru_utime), tv(ru.ru_stime), rss_mb)
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
    std::path::Path::new(argv0).file_name() == Some(OsStr::new("rustc"))
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
