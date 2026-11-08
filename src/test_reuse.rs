//! Reuse the result of an identical passing `test` run instead of running it
//! again.
//!
//! Agents often rerun the exact same `justrust test ...` with nothing changed
//! (to see output again, after an unrelated thought, or to double check).
//! When the command, its environment, the justrust binary and every source
//! file the build can read are exactly as they were at the start of a run
//! that passed, the result is known: this prints that run's results and exits
//! 0 without syncing, compiling or running anything.
//!
//! The key is captured before the build starts and records each file's size,
//! mode and mtime in nanoseconds. Any edit after that point, including one
//! during the stored run, changes a stamp, so a later run can match only a
//! tree nobody touched since the stored run started. Files outside the source
//! roots (gitignored files, `/tmp`, the network) are not covered: tests that
//! depend on them should run with `--rerun` or `JUSTRUST_TEST_REUSE=0`.
//!
//! Only passing runs are reused, for at most `MAX_AGE_SECS`. Scanning the
//! tree costs time (26 ms for 4,000 files, 600 ms for 95,000), so a command
//! is scanned only when its tests took at least `SCAN_PAYBACK` times the scan.

use crate::depcache::H128;
use crate::summary::Summary;
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Instant;

/// A passing result older than this is run again.
const MAX_AGE_SECS: f64 = 12.0 * 3600.0;
/// Scan only when the tests take this many times longer than the scan.
const SCAN_PAYBACK: f64 = 20.0;
/// With no history, scan when the last scan of this directory took at most this.
const FIRST_SCAN_MS: f64 = 100.0;
/// Flag that skips reuse for one run (removed before cargo sees it).
pub const RERUN_FLAG: &str = "--rerun";

/// Environment that differs between shells and sessions but cannot change
/// what a test computes.
fn volatile_env(k: &str) -> bool {
    k.starts_with("JCODE_")
        || k.starts_with("SSH_")
        || k.starts_with("JUSTRUST_RUN_ID")
        || k.starts_with("BASH_FUNC_")
        || matches!(
            k,
            "_" | "PWD"
                | "OLDPWD"
                | "SHLVL"
                | "MOTD_SHOWN"
                | "XDG_SESSION_ID"
                | "DBUS_SESSION_BUS_ADDRESS"
                | "TERM_SESSION_ID"
                | "WINDOWID"
                | "which_declare"
        )
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Entry {
    /// Tree key at the start of the run, when it was scanned.
    tree: Option<String>,
    run_id: String,
    at: f64,
    passed: bool,
    tests_passed: u64,
    /// What the caller waited for.
    wall: f64,
    test_secs: f64,
    scan_ms: f64,
}

/// This run's key, kept until the run is recorded.
struct Pending {
    cheap: String,
    tree: Option<String>,
    scan_ms: f64,
}

static PENDING: OnceLock<Pending> = OnceLock::new();

fn dir() -> Option<PathBuf> {
    crate::paths::home().ok().map(|h| h.join("test-reuse"))
}

fn enabled() -> bool {
    !std::env::var_os("JUSTRUST_TEST_REUSE").is_some_and(|v| v == "0")
        && std::env::var_os(crate::route::ON_AGENT).is_none()
}

/// Whether this cargo command's result can be reused at all.
fn eligible(full: &[OsString]) -> bool {
    let Some(args) = full
        .iter()
        .map(|a| a.to_str())
        .collect::<Option<Vec<&str>>>()
    else {
        return false;
    };
    if !matches!(args.first(), Some(&"test" | &"t")) {
        return false;
    }
    // Before `--`: cargo flags. Outputs other than a test result, or a
    // result the caller parses itself, are not replayed.
    let cargo_args = args.iter().take_while(|a| **a != "--");
    !cargo_args.clone().any(|a| {
        *a == "--no-run"
            || a.starts_with("--message-format")
            || a.starts_with("--timings")
            || *a == "--list"
    }) && !args
        .iter()
        .any(|a| *a == "--list" || a.starts_with("--format"))
}

fn cheap_key(full: &[OsString], cwd: &Path) -> String {
    let mut h = H128::new();
    h.field(b"test-reuse-v1");
    h.field(env!("CARGO_PKG_VERSION").as_bytes());
    if let Ok(exe) = std::env::current_exe()
        && let Ok(m) = std::fs::metadata(&exe)
    {
        let (s, ns) = crate::sys::mtime(&m);
        h.field(format!("{}:{s}.{ns}", m.len()).as_bytes());
    }
    h.field(cwd.as_os_str().as_encoded_bytes());
    for a in full {
        h.field(a.as_encoded_bytes());
    }
    let mut env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.into_string().ok()?)))
        .filter(|(k, _)| !volatile_env(k))
        .collect();
    env.sort();
    for (k, v) in env {
        h.field(k.as_bytes());
        h.field(v.as_bytes());
    }
    h.hex()
}

/// Key of every file the build can read under the source roots.
fn tree_key(cwd: &Path) -> Option<String> {
    let roots = crate::remote_build::source_roots(cwd).ok()?;
    let mut h = H128::new();
    for root in roots {
        let scan = crate::remote_sync::scan(&root).ok()?;
        h.field(root.as_os_str().as_encoded_bytes());
        for (rel, st) in &scan.files {
            h.field(rel.as_os_str().as_encoded_bytes());
            h.field(&st.mtime_ns.to_le_bytes());
            h.field(&st.size.to_le_bytes());
            h.field(&st.mode.to_le_bytes());
        }
        for rel in &scan.oversized {
            let m = std::fs::symlink_metadata(root.join(rel)).ok()?;
            let (s, ns) = crate::sys::mtime(&m);
            h.field(rel.as_os_str().as_encoded_bytes());
            h.field(format!("{}:{s}.{ns}", m.len()).as_bytes());
        }
    }
    Some(h.hex())
}

fn cwd_stats_path(d: &Path, cwd: &Path) -> PathBuf {
    let mut h = H128::new();
    h.field(cwd.as_os_str().as_encoded_bytes());
    d.join(format!("scan-{}.ms", &h.hex()[..16]))
}

fn load(d: &Path, cheap: &str) -> Option<Entry> {
    serde_json::from_slice(&std::fs::read(d.join(format!("{cheap}.json"))).ok()?).ok()
}

/// Whether to pay for a scan given what is known about this command.
fn should_scan(entry: Option<&Entry>, cwd_scan_ms: Option<f64>) -> bool {
    match entry {
        Some(e) => {
            let scan = if e.scan_ms > 0.0 {
                e.scan_ms
            } else {
                cwd_scan_ms.unwrap_or(0.0)
            };
            e.test_secs * 1000.0 >= SCAN_PAYBACK * scan
        }
        None => cwd_scan_ms.is_none_or(|ms| ms <= FIRST_SCAN_MS),
    }
}

/// Whether `entry` is a passing result for `tree` still young enough to reuse.
fn reusable(entry: &Entry, tree: &str, now: f64) -> bool {
    entry.passed && entry.tree.as_deref() == Some(tree) && now - entry.at <= MAX_AGE_SECS
}

/// Called before routing. Returns the exit code when an identical passing
/// run was reused; `None` runs the build (and remembers the key for `store`).
pub fn try_reuse(full: &[OsString]) -> Option<i32> {
    if !enabled() || !eligible(full) {
        return None;
    }
    let t0 = Instant::now();
    let cwd = std::env::current_dir().ok()?;
    let d = dir()?;
    let cheap = cheap_key(full, &cwd);
    let entry = load(&d, &cheap);
    let cwd_scan_ms = std::fs::read_to_string(cwd_stats_path(&d, &cwd))
        .ok()
        .and_then(|s| s.trim().parse().ok());
    let (tree, scan_ms) = if should_scan(entry.as_ref(), cwd_scan_ms) {
        let s = Instant::now();
        let tree = tree_key(&cwd);
        let ms = s.elapsed().as_secs_f64() * 1000.0;
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::write(cwd_stats_path(&d, &cwd), format!("{ms:.1}"));
        (tree, ms)
    } else {
        (None, 0.0)
    };
    if let (Some(e), Some(t)) = (&entry, &tree)
        && reusable(e, t, crate::paths::now())
        && replay(e, t0)
    {
        return Some(0);
    }
    let _ = PENDING.set(Pending {
        cheap,
        tree,
        scan_ms,
    });
    None
}

/// Print the stored run's results. False when its output is gone.
fn replay(e: &Entry, t0: Instant) -> bool {
    let Ok(runs) = crate::paths::runs_dir() else {
        return false;
    };
    let Ok(raw) = std::fs::read_to_string(runs.join(&e.run_id).join("output.jsonl")) else {
        return false;
    };
    let mut shown = Vec::new();
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(l) = v["l"].as_str() else { continue };
        let t = l.trim_start();
        if t.starts_with("Running ") || t.starts_with("Doc-tests ") || t.starts_with("test result:")
        {
            shown.push(l.to_owned());
        }
    }
    if shown.is_empty() {
        return false;
    }
    for l in &shown {
        eprintln!("{l}");
    }
    let age = (crate::paths::now() - e.at).max(0.0);
    let overhead = t0.elapsed().as_secs_f64();
    eprintln!(
        "\njustrust: ok: {} test{} passed (reused run {} from {} ago: same command, environment and \
         sources, nothing changed since it started; it took {:.1}s, this took {:.2}s). \
         `--rerun` runs the tests again",
        e.tests_passed,
        if e.tests_passed == 1 { "" } else { "s" },
        e.run_id,
        human_age(age),
        e.wall,
        overhead
    );
    if let Some(d) = dir() {
        let rec = serde_json::json!({
            "at": crate::paths::now(),
            "cwd": std::env::current_dir().ok(),
            "reused": e.run_id,
            "saved_secs": e.wall,
            "overhead_secs": overhead,
        });
        let _ = crate::paths::append_line(&d.join("hits.jsonl"), &rec.to_string());
    }
    true
}

fn human_age(secs: f64) -> String {
    if secs < 120.0 {
        format!("{secs:.0}s")
    } else if secs < 7200.0 {
        format!("{:.0} min", secs / 60.0)
    } else {
        format!("{:.1} h", secs / 3600.0)
    }
}

/// After a recorded run: remember its result under the key captured at its
/// start. `wall` is what the caller waited for.
pub fn store(s: &Summary, wall: f64) {
    let Some(p) = PENDING.get() else { return };
    let Some(d) = dir() else { return };
    let passed = s.exit == 0 && !s.diagnostics.compile_failed && s.tests.failed == 0;
    let e = Entry {
        tree: p.tree.clone(),
        run_id: s.id.clone(),
        at: s.start,
        passed: passed && s.tests.passed > 0,
        tests_passed: s.tests.passed,
        wall,
        test_secs: s.phases.test_run,
        scan_ms: p.scan_ms,
    };
    let _ = std::fs::create_dir_all(&d);
    let path = d.join(format!("{}.json", p.cheap));
    let tmp = d.join(format!("{}.json.{}", p.cheap, std::process::id()));
    if let Ok(b) = serde_json::to_vec(&e)
        && std::fs::write(&tmp, b).is_ok()
    {
        let _ = std::fs::rename(&tmp, &path);
    }
}

/// Remove `--rerun` (before `--`) and report whether it was given.
pub fn take_rerun_flag(args: Vec<OsString>) -> (Vec<OsString>, bool) {
    let mut out = Vec::with_capacity(args.len());
    let mut rerun = false;
    let mut passed = false;
    for a in args {
        passed |= a == "--";
        if !passed && a == RERUN_FLAG {
            rerun = true;
        } else {
            out.push(a);
        }
    }
    (out, rerun)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(v: &[&str]) -> Vec<OsString> {
        v.iter().map(OsString::from).collect()
    }

    #[test]
    fn only_plain_test_runs_are_eligible() {
        assert!(eligible(&os(&["test"])));
        assert!(eligible(&os(&[
            "test",
            "--lib",
            "--",
            "foo",
            "--test-threads=4"
        ])));
        assert!(!eligible(&os(&["check"])));
        assert!(!eligible(&os(&["build"])));
        assert!(!eligible(&os(&["test", "--no-run"])));
        assert!(!eligible(&os(&["test", "--message-format=json"])));
        assert!(!eligible(&os(&["test", "--", "--list"])));
        assert!(!eligible(&os(&["test", "--", "--format", "json"])));
    }

    #[test]
    fn rerun_flag_is_removed_only_before_the_separator() {
        let (a, r) = take_rerun_flag(os(&["--lib", "--rerun", "--", "--rerun"]));
        assert!(r);
        assert_eq!(a, os(&["--lib", "--", "--rerun"]));
        let (a, r) = take_rerun_flag(os(&["--lib"]));
        assert!(!r);
        assert_eq!(a, os(&["--lib"]));
    }

    #[test]
    fn scans_only_when_the_tests_pay_for_it() {
        let e = |test_secs, scan_ms| Entry {
            test_secs,
            scan_ms,
            ..Default::default()
        };
        // Unknown command: scan unless this directory is known to be slow.
        assert!(should_scan(None, None));
        assert!(should_scan(None, Some(30.0)));
        assert!(!should_scan(None, Some(600.0)));
        // 250 s of tests easily pays for a 30 ms scan; 5 s does not pay 600 ms.
        assert!(should_scan(Some(&e(250.0, 30.0)), None));
        assert!(!should_scan(Some(&e(5.0, 600.0)), None));
        assert!(should_scan(Some(&e(13.0, 600.0)), None));
        // Scan time unknown for this command: fall back to the directory's.
        assert!(!should_scan(Some(&e(5.0, 0.0)), Some(600.0)));
    }

    #[test]
    fn reuses_only_young_passing_results_for_the_same_tree() {
        let e = Entry {
            tree: Some("t".into()),
            at: 1000.0,
            passed: true,
            ..Default::default()
        };
        assert!(reusable(&e, "t", 1000.0 + 60.0));
        assert!(!reusable(&e, "u", 1060.0));
        assert!(!reusable(&e, "t", 1000.0 + MAX_AGE_SECS + 1.0));
        let failed = Entry {
            passed: false,
            ..e.clone()
        };
        assert!(!reusable(&failed, "t", 1060.0));
        let unscanned = Entry { tree: None, ..e };
        assert!(!reusable(&unscanned, "t", 1060.0));
    }

    #[test]
    fn session_noise_does_not_change_the_key_but_real_env_does() {
        assert!(volatile_env("JCODE_SESSION_ID"));
        assert!(volatile_env("SSH_CONNECTION"));
        assert!(volatile_env("OLDPWD"));
        assert!(!volatile_env("RUST_TEST_THREADS"));
        assert!(!volatile_env("PATH"));
        assert!(!volatile_env("JUSTRUST_REMOTE"));
    }

    #[test]
    fn tree_key_changes_on_any_edit_and_not_otherwise() {
        let d = std::env::temp_dir().join(format!("jr-reuse-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("src")).unwrap();
        std::fs::write(
            d.join("Cargo.toml"),
            "[package]\nname=\"r\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
        )
        .unwrap();
        std::fs::write(d.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
        let scan_key = || {
            let scan = crate::remote_sync::scan(&d).unwrap();
            let mut h = H128::new();
            for (rel, st) in &scan.files {
                h.field(rel.as_os_str().as_encoded_bytes());
                h.field(&st.mtime_ns.to_le_bytes());
                h.field(&st.size.to_le_bytes());
            }
            h.hex()
        };
        let a = scan_key();
        assert_eq!(a, scan_key());
        std::thread::sleep(std::time::Duration::from_millis(20));
        // Same size, new content: the mtime still changes the key.
        std::fs::write(d.join("src/lib.rs"), "pub fn g() {}\n").unwrap();
        assert_ne!(a, scan_key());
        let _ = std::fs::remove_dir_all(&d);
    }
}
