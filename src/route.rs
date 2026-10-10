//! Automatic local vs remote routing for `justrust check|clippy|test`.
//!
//! Every agent-mode build asks [`maybe_remote`] first. It returns (build
//! locally) unless a remote run is predicted to be faster, in which case it
//! runs the build remotely and exits with its code.
//!
//! Robustness first. Local is the default and costs nothing extra:
//! - `JUSTRUST_REMOTE=0|off`: always local, nothing is read.
//! - `build` and `run` are always local (their artifacts are needed here).
//! - No backend configured, or the machine is not known to be up: local,
//!   with no network call, no daemon start, and no machine start.
//! - `JUSTRUST_REMOTE=1|force`: remote for check, clippy and test, starting
//!   the machine if needed.
//!
//! The prediction uses the run index: for this cwd and these exact args,
//! the median of the last few local walls against the median of the last
//! few remote client walls (what the caller actually waited, sync and ssh
//! included). With no recent remote sample, a command whose local median
//! is over [`EXPLORE_LOCAL_SECS`] tries remote once to get one. Remote
//! samples expire after [`REMOTE_MAX_AGE`], so a command that lost on
//! remote is retried now and then.
//!
//! Any infrastructure failure falls back to a local build, says so in one
//! line, and pauses auto routing for [`FAIL_BACKOFF`] seconds.

use std::ffi::OsString;
use std::path::Path;

use crate::remote_backend::{self, Backend};
use crate::remote_build::{self, Infra};
use crate::summary::IndexEntry;

/// Local median above which a command with no remote sample tries remote.
const EXPLORE_LOCAL_SECS: f64 = 3.0;
/// Samples per side used for the median.
const RECENT: usize = 5;
/// Local samples older than this are ignored (seconds).
const LOCAL_MAX_AGE: f64 = 14.0 * 86400.0;
/// Remote samples older than this are ignored, so remote gets re-explored.
const REMOTE_MAX_AGE: f64 = 2.0 * 86400.0;
/// Remote must win by this factor and by at least [`MIN_GAIN_SECS`].
const WIN_FACTOR: f64 = 0.8;
const MIN_GAIN_SECS: f64 = 0.5;
/// Fixed client cost of a remote call when the client wall is unknown.
const REMOTE_OVERHEAD_SECS: f64 = 0.6;
/// After an infrastructure failure, auto routing stays local this long.
const FAIL_BACKOFF: f64 = 600.0;
/// The AWS machine stops itself after this many idle minutes by default.
const DEFAULT_IDLE_MINUTES: f64 = 30.0;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Mode {
    Off,
    Auto,
    Force,
}

pub fn mode() -> Mode {
    mode_from(std::env::var("JUSTRUST_REMOTE").ok().as_deref())
}

fn mode_from(v: Option<&str>) -> Mode {
    match v.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("0" | "off" | "no" | "false" | "local") => Mode::Off,
        Some("1" | "force" | "on" | "yes" | "true" | "remote") => Mode::Force,
        _ => Mode::Auto,
    }
}

/// Subcommands that may run remotely.
fn routable(sub: &str) -> bool {
    matches!(sub, "check" | "clippy" | "test")
}

#[derive(Debug, Clone, PartialEq)]
pub enum Choice {
    Local(String),
    Remote(String),
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.total_cmp(b));
    let n = v.len();
    Some(if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    })
}

/// The routing decision for `args` (subcommand first) run in `cwd`, from
/// the index entries. `client_wall` gives what the caller waited for a
/// remote entry. Pure, so it can be tested with synthetic history.
pub fn decide(
    args: &[String],
    cwd: &str,
    entries: &[IndexEntry],
    client_wall: &dyn Fn(&IndexEntry) -> f64,
    now: f64,
) -> Choice {
    let Some(sub) = args.first() else {
        return Choice::Local("no subcommand".into());
    };
    if !routable(sub) {
        return Choice::Local(format!("`{sub}` always runs locally"));
    }
    let same: Vec<&IndexEntry> = entries
        .iter()
        .filter(|e| e.cwd == cwd && e.args == args && e.wall > 0.0)
        // Interrupted runs say nothing about cost.
        .filter(|e| !matches!(e.exit, 130 | 137 | 143))
        .collect();
    let recent = |remote: bool, max_age: f64| -> Vec<&IndexEntry> {
        let mut v: Vec<&IndexEntry> = same
            .iter()
            .copied()
            .filter(|e| e.remote.is_some() == remote && now - e.start <= max_age)
            .collect();
        v.sort_by(|a, b| b.start.total_cmp(&a.start));
        v.truncate(RECENT);
        v
    };
    let local = median(
        recent(false, LOCAL_MAX_AGE)
            .iter()
            .map(|e| e.wall)
            .collect(),
    );
    let remote = median(
        recent(true, REMOTE_MAX_AGE)
            .iter()
            .map(|e| client_wall(e))
            .collect(),
    );
    match (local, remote) {
        (None, None) => Choice::Local("no history for this command".into()),
        (Some(l), None) if l > EXPLORE_LOCAL_SECS => Choice::Remote(format!(
            "trying remote: local ~{l:.1}s, no recent remote sample for this command"
        )),
        (Some(l), None) => Choice::Local(format!(
            "local ~{l:.1}s, too short to be worth trying remote"
        )),
        // Remote-only history (local samples aged out): stay remote while
        // it is cheap, else take a fresh local sample.
        (None, Some(r)) if r <= EXPLORE_LOCAL_SECS => {
            Choice::Remote(format!("remote ~{r:.1}s, no recent local sample"))
        }
        (None, Some(r)) => Choice::Local(format!(
            "remote ~{r:.1}s, taking a fresh local sample for this command"
        )),
        (Some(l), Some(r)) if r < l * WIN_FACTOR && l - r >= MIN_GAIN_SECS => Choice::Remote(
            format!("local ~{l:.1}s vs remote ~{r:.1}s for this command"),
        ),
        (Some(l), Some(r)) => Choice::Local(format!(
            "local ~{l:.1}s vs remote ~{r:.1}s for this command"
        )),
    }
}

/// Local median above which a signed-in hosted user may start the machine.
const HOSTED_START_LOCAL_SECS: f64 = 20.0;

/// Median local wall for exactly these args in `cwd`, recent samples only.
fn local_median(args: &[String], cwd: &str, entries: &[IndexEntry], now: f64) -> Option<f64> {
    let mut v: Vec<&IndexEntry> = entries
        .iter()
        .filter(|e| e.cwd == cwd && e.args == args && e.wall > 0.0 && e.remote.is_none())
        .filter(|e| !matches!(e.exit, 130 | 137 | 143) && now - e.start <= LOCAL_MAX_AGE)
        .collect();
    v.sort_by(|a, b| b.start.total_cmp(&a.start));
    v.truncate(RECENT);
    median(v.iter().map(|e| e.wall).collect())
}

/// Hosted, machine not up: worth starting it for this build? Only when
/// the local build is predicted slow, or there is no local sample and the
/// workspace has never been built here.
pub fn hosted_start_worth_it(local: Option<f64>, cold_target: bool) -> Option<String> {
    match local {
        Some(l) if l > HOSTED_START_LOCAL_SECS => {
            Some(format!("local ~{l:.0}s, starting the hosted build machine"))
        }
        None if cold_target => {
            Some("cold target dir, no local sample: starting the hosted build machine".into())
        }
        _ => None,
    }
}

/// No `target/` with build output next to the workspace's Cargo.lock.
fn cold_target(cwd: &Path) -> bool {
    if let Some(t) = std::env::var_os("CARGO_TARGET_DIR") {
        return !Path::new(&t).join("debug").exists();
    }
    let Some(root) = cwd
        .ancestors()
        .find(|d| d.join("Cargo.lock").exists())
        .or_else(|| cwd.ancestors().find(|d| d.join("Cargo.toml").exists()))
    else {
        return false;
    };
    !root.join("target/debug").exists()
}

/// Index entries for `cwd` only. Lines for other directories are skipped
/// before parsing, so this stays cheap on a large index.
fn load_entries(cwd: &str) -> Vec<IndexEntry> {
    let Ok(path) = crate::paths::index_file() else {
        return Vec::new();
    };
    let Ok(s) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let needle = format!("\"cwd\":{}", serde_json::to_string(cwd).unwrap_or_default());
    s.lines()
        .filter(|l| l.contains(&needle))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// What the client waited for a remote run, from its local summary. Sync
/// time beyond [`SYNC_ALLOWANCE_SECS`] is a one-time cost (the first mirror
/// of a repository, or after a reconnect): the daemon pushes later edits as
/// they are saved, before the build asks, so it is not charged again.
fn remote_client_wall(e: &IndexEntry) -> f64 {
    let remote = crate::paths::runs_dir()
        .ok()
        .and_then(|d| std::fs::read(d.join(&e.id).join("summary.json")).ok())
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .map(|v| v["remote"].clone());
    let Some(r) = remote else {
        return e.wall + REMOTE_OVERHEAD_SECS;
    };
    match r["client_wall"].as_f64().filter(|w| *w > 0.0) {
        Some(w) => {
            let sync = r["sync_ms"].as_f64().unwrap_or(0.0) / 1000.0;
            (w - (sync - SYNC_ALLOWANCE_SECS).max(0.0)).max(e.wall)
        }
        None => e.wall + REMOTE_OVERHEAD_SECS,
    }
}

/// Sync on the critical path that is charged to every remote run.
const SYNC_ALLOWANCE_SECS: f64 = 0.2;

fn file_mtime(p: &Path) -> Option<f64> {
    std::fs::metadata(p)
        .ok()?
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs_f64())
}

/// The AWS machine stops itself when idle without telling us, leaving a
/// stale cached IP. Starting it is never acceptable in auto mode, so only
/// trust the IP if something shows the machine alive recently enough: the
/// last status probe (with its idle countdown), the last remote run, or the
/// IP being written.
fn aws_recently_alive(now: f64, last_remote_run: Option<f64>) -> bool {
    let Ok(dir) = crate::remote::dir() else {
        return false;
    };
    let idle_secs = std::fs::read(dir.join("state.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["idle_minutes"].as_f64())
        .filter(|m| *m > 0.0)
        .unwrap_or(DEFAULT_IDLE_MINUTES)
        * 60.0;
    let mut until = 0.0f64;
    if let Some(t) = file_mtime(&dir.join("ip")) {
        until = until.max(t + idle_secs);
    }
    if let Some(t) = last_remote_run {
        until = until.max(t + idle_secs);
    }
    if let Some(p) = std::fs::read(dir.join("probe.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        && p["ok"].as_bool() == Some(true)
        && let Some(at) = p["at"].as_f64()
    {
        let left = p["idle_left_min"].as_f64().unwrap_or(0.0) * 60.0;
        until = until.max(at + left.max(0.0));
    }
    now < until
}

fn fail_file() -> Option<std::path::PathBuf> {
    crate::remote::dir().ok().map(|d| d.join("route-fail"))
}

fn recently_failed(now: f64) -> bool {
    fail_file()
        .and_then(|f| file_mtime(&f))
        .is_some_and(|t| now - t < FAIL_BACKOFF)
}

fn explain() -> bool {
    matches!(
        std::env::var("JUSTRUST_REMOTE_EXPLAIN").ok().as_deref(),
        Some("1" | "true" | "yes")
    )
}

fn say_local(why: &str) {
    if explain() {
        eprintln!("justrust: local ({why})");
    }
}

/// Route one agent-mode build. Returns when the build should run locally;
/// otherwise runs it remotely and exits with its code.
pub fn maybe_remote(full: &[OsString]) {
    let m = mode();
    if m == Mode::Off {
        return say_local("JUSTRUST_REMOTE=off");
    }
    // The remote agent runs this same binary: never route from there.
    if std::env::var_os("JUSTRUST_RUN_ID_OVERRIDE").is_some() {
        return;
    }
    let Some(args) = full
        .iter()
        .map(|a| a.to_str().map(str::to_string))
        .collect::<Option<Vec<String>>>()
    else {
        return say_local("non-UTF-8 arguments");
    };
    let Some(sub) = args.first() else { return };
    if !routable(sub) {
        return say_local(&format!("`{sub}` always runs locally"));
    }
    let Some(backend) = remote_backend::load() else {
        if m == Mode::Force {
            eprintln!(
                "justrust: JUSTRUST_REMOTE=force but no remote machine is configured, building locally"
            );
        }
        return say_local("no remote machine configured");
    };
    let now = crate::paths::now();
    let reason = if m == Mode::Force {
        "JUSTRUST_REMOTE=force".to_string()
    } else {
        if recently_failed(now) {
            return say_local("remote failed in the last 10 minutes");
        }
        let Ok(cwd_path) = std::env::current_dir() else {
            return;
        };
        let cwd = cwd_path.to_string_lossy().into_owned();
        let entries = load_entries(&cwd);
        let mut start_reason = None;
        if !backend.probably_up() {
            if backend == Backend::Hosted && crate::remote_hosted::signed_in() {
                start_reason = hosted_start_worth_it(
                    local_median(&args, &cwd, &entries, now),
                    cold_target(&cwd_path),
                );
            }
            if start_reason.is_none() {
                return say_local("remote machine not running");
            }
        }
        if backend == Backend::Aws {
            let last_remote = entries
                .iter()
                .filter(|e| e.remote.is_some())
                .map(|e| e.start + e.wall)
                .reduce(f64::max);
            if !aws_recently_alive(now, last_remote) {
                return say_local("remote machine probably stopped itself (idle)");
            }
        }
        match (
            start_reason,
            decide(&args, &cwd, &entries, &remote_client_wall, now),
        ) {
            (Some(why), _) => why,
            (None, Choice::Local(why)) => return say_local(&why),
            (None, Choice::Remote(why)) => why,
        }
    };
    eprintln!("justrust: remote ({reason})");
    let start = m == Mode::Force || backend == Backend::Hosted;
    match remote_build::run_remote(sub, &args[1..], start) {
        Ok(Ok(o)) => std::process::exit(o.code),
        Ok(Err(Infra(msg))) => {
            if let Some(f) = fail_file() {
                let _ = std::fs::write(f, format!("{now} {msg}\n"));
            }
            if Infra(msg.clone()).is_billing() {
                eprintln!("{msg}");
            } else {
                eprintln!("justrust: remote unavailable ({msg}), building locally");
            }
        }
        Err(e) => {
            // Output may already have been shown: a local rerun would
            // duplicate it.
            eprintln!("justrust: remote build failed: {e:#}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: f64 = 1_000_000.0;
    const CWD: &str = "/w";

    fn e(args: &[&str], wall: f64, remote: bool, age: f64) -> IndexEntry {
        IndexEntry {
            id: format!("{wall}-{age}"),
            start: NOW - age,
            cwd: CWD.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            exit: 0,
            wall,
            cpu_secs: 0.0,
            compile: 0.0,
            test_run: 0.0,
            units_compiled: 0,
            top_unit: None,
            compile_failed: false,
            agent_session: None,
            remote: remote.then(|| "m".to_string()),
        }
    }

    fn a(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    fn run(args: &[&str], entries: &[IndexEntry]) -> Choice {
        // Remote client wall = index wall + fixed overhead.
        decide(
            &a(args),
            CWD,
            entries,
            &|e| e.wall + REMOTE_OVERHEAD_SECS,
            NOW,
        )
    }

    fn is_remote(c: &Choice) -> bool {
        matches!(c, Choice::Remote(_))
    }

    #[test]
    fn hosted_start_rule() {
        assert!(hosted_start_worth_it(Some(25.0), false).is_some());
        assert!(hosted_start_worth_it(Some(10.0), true).is_none());
        assert!(hosted_start_worth_it(None, true).is_some());
        assert!(hosted_start_worth_it(None, false).is_none());
    }

    #[test]
    fn modes() {
        assert_eq!(mode_from(None), Mode::Auto);
        assert_eq!(mode_from(Some("auto")), Mode::Auto);
        assert_eq!(mode_from(Some("0")), Mode::Off);
        assert_eq!(mode_from(Some("OFF")), Mode::Off);
        assert_eq!(mode_from(Some("1")), Mode::Force);
        assert_eq!(mode_from(Some("force")), Mode::Force);
    }

    #[test]
    fn build_and_run_stay_local() {
        let h = vec![
            e(&["build"], 60.0, false, 10.0),
            e(&["run"], 60.0, false, 10.0),
        ];
        assert!(!is_remote(&run(&["build"], &h)));
        assert!(!is_remote(&run(&["run"], &h)));
    }

    #[test]
    fn no_history_is_local() {
        assert!(!is_remote(&run(&["check"], &[])));
    }

    #[test]
    fn explores_slow_command_without_remote_sample() {
        let h = vec![
            e(&["test"], 9.0, false, 100.0),
            e(&["test"], 10.0, false, 50.0),
        ];
        assert!(is_remote(&run(&["test"], &h)));
        // Fast command: not worth exploring.
        let h = vec![e(&["check"], 0.4, false, 50.0)];
        assert!(!is_remote(&run(&["check"], &h)));
    }

    #[test]
    fn remote_wins_when_clearly_faster() {
        let h = vec![
            e(&["test", "-p", "x"], 9.0, false, 300.0),
            e(&["test", "-p", "x"], 9.4, false, 200.0),
            e(&["test", "-p", "x"], 1.7, true, 100.0),
        ];
        match run(&["test", "-p", "x"], &h) {
            Choice::Remote(why) => assert!(why.contains("local ~9.2s vs remote ~2.3s"), "{why}"),
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn local_wins_on_one_line_edits() {
        // About equal: the remote fixed cost tips it to local.
        let h = vec![
            e(&["check"], 1.0, false, 300.0),
            e(&["check"], 1.1, false, 200.0),
            e(&["check"], 0.6, true, 100.0),
        ];
        assert!(!is_remote(&run(&["check"], &h)));
    }

    #[test]
    fn other_args_and_dirs_do_not_count() {
        let mut other_dir = e(&["test"], 1.0, true, 10.0);
        other_dir.cwd = "/elsewhere".into();
        let h = vec![
            e(&["test"], 9.0, false, 100.0),
            e(&["test", "-p", "y"], 1.0, true, 10.0),
            other_dir,
        ];
        // Still exploring: no remote sample for exactly this command here.
        match run(&["test"], &h) {
            Choice::Remote(why) => assert!(why.contains("trying remote"), "{why}"),
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn stale_remote_samples_are_re_explored() {
        let h = vec![
            e(&["test"], 9.0, false, 100.0),
            // Lost long ago: expired, so remote gets another try.
            e(&["test"], 20.0, true, REMOTE_MAX_AGE + 10.0),
        ];
        assert!(is_remote(&run(&["test"], &h)));
        // Lost recently: stay local.
        let h = vec![
            e(&["test"], 9.0, false, 100.0),
            e(&["test"], 20.0, true, 50.0),
        ];
        assert!(!is_remote(&run(&["test"], &h)));
    }

    #[test]
    fn median_resists_outliers_and_uses_recent_samples() {
        let mut h: Vec<IndexEntry> = (0..5)
            .map(|i| e(&["check"], 8.0, false, 100.0 + i as f64))
            .collect();
        h.push(e(&["check"], 120.0, false, 50.0)); // contended outlier
        // Old samples beyond the last RECENT are ignored.
        h.extend((0..10).map(|i| e(&["check"], 0.3, false, 10_000.0 + i as f64)));
        h.push(e(&["check"], 1.0, true, 20.0));
        match run(&["check"], &h) {
            Choice::Remote(why) => assert!(why.contains("local ~8.0s"), "{why}"),
            c => panic!("{c:?}"),
        }
    }

    #[test]
    fn interrupted_runs_are_ignored() {
        let mut x = e(&["test"], 50.0, false, 10.0);
        x.exit = 130;
        assert!(!is_remote(&run(&["test"], &[x])));
    }

    #[test]
    fn remote_only_history() {
        let h = vec![e(&["test"], 1.0, true, 10.0)];
        assert!(is_remote(&run(&["test"], &h)));
        let h = vec![e(&["test"], 30.0, true, 10.0)];
        assert!(!is_remote(&run(&["test"], &h)));
    }
}
