//! The rustc shim. Cargo runs it as `RUSTC` (`~/.justrust/bin/shim/rustc`) while
//! a recorded run is active. For each compilation unit it records wall time,
//! CPU time, peak memory, when metadata (`.rmeta`) became available, and, when
//! it is safe, rustc's own per-pass timings.
//!
//! Incremental-cache safety: rustc keys its incremental cache on whether
//! unstable options are allowed (`RUSTC_BOOTSTRAP`), but not on `-Ztime-passes`.
//! So the shim only adds `-Ztime-passes` when the unit already runs with
//! `RUSTC_BOOTSTRAP=1` (for example via the Jcode Desktop parallel-frontend
//! wrapper). Other units are timed from the outside only. Set
//! `JUSTRUST_PASSES=always` to force pass timing everywhere, at the cost of one
//! cold incremental rebuild whenever the mode flips.

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
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    let real = real_rustc();
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

fn want_passes(args: &[OsString]) -> (bool, bool) {
    let has = args
        .iter()
        .any(|a| a.to_string_lossy().starts_with("-Ztime-passes"));
    if has {
        return (false, false);
    }
    match std::env::var("JUSTRUST_PASSES").as_deref() {
        Ok("0") | Ok("off") | Ok("never") => (false, false),
        Ok("always") => (true, true),
        _ => (
            std::env::var("RUSTC_BOOTSTRAP").as_deref() == Ok("1"),
            false,
        ),
    }
}

/// Returns `None` only if the compiler could not be spawned at all.
fn run_unit(real: &PathBuf, args: &[OsString], run_dir: &std::path::Path) -> Option<i32> {
    let (passes_on, force_bootstrap) = want_passes(args);
    let mut cmd = Command::new(real);
    cmd.args(args).stderr(Stdio::piped());
    if passes_on {
        cmd.arg("-Ztime-passes").arg("-Ztime-passes-format=json");
    }
    if force_bootstrap {
        cmd.env("RUSTC_BOOTSTRAP", "1");
    }
    let start = paths::now();
    let mut child = cmd.spawn().ok()?;
    let stderr = child.stderr.take()?;
    let rmeta_at: Arc<Mutex<Option<f64>>> = Arc::default();
    let passes: Arc<Mutex<Vec<Pass>>> = Arc::default();
    let reader = {
        let rmeta_at = rmeta_at.clone();
        let passes = passes.clone();
        std::thread::spawn(move || forward_stderr(stderr, passes_on, &rmeta_at, &passes))
    };

    let (exit, user, sys, max_rss_mb) = wait4(child.id() as i32);
    let end = paths::now();
    let _ = reader.join();

    let mut unit = describe(args);
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
                passes.lock().unwrap().push(p);
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
    (
        code,
        tv(ru.ru_utime),
        tv(ru.ru_stime),
        ru.ru_maxrss as f64 / 1024.0,
    )
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
