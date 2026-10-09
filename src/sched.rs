//! Machine-wide CPU priority between concurrent builds (Linux, cgroup v2).
//!
//! Parallel agents each start cargo with `-j<ncpu>`, so five builds on a
//! 16-thread machine run ~80 rustc threads that the kernel shares equally.
//! A one-line edit build (one unit, ~100 CPU-s) then waits behind cold builds
//! that need 1,000+ CPU-s, and its wall time goes from 9 s to 40-200 s.
//!
//! Every recorded run moves itself, before it starts cargo, into its own
//! transient systemd scope (`justrust-<run id>.scope` in `app-justrust.slice`,
//! created over the user bus with `busctl`), so cargo and every rustc, linker,
//! build script and test binary it starts are inside it. While the build
//! runs, the scope's `cpu.weight` follows the CPU time it has used so far
//! (least attained service): a build that just started has weight 1000, one
//! that has burned 20 CPU-minutes about 50. The kernel then gives short builds
//! most of the CPU when they compete with long ones and still lets long builds
//! use every idle core. Nothing is throttled on an uncontended machine.
//!
//! All builds share the one slice, so together they compete with the rest of
//! the desktop like a single app.
//!
//! Fails open: no systemd user bus, no `busctl`, a slow move, or
//! `JUSTRUST_SCHED=0` just run cargo where justrust already runs.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub const SLICE: &str = "app-justrust.slice";
const MAX_WEIGHT: u64 = 1000;
const MIN_WEIGHT: u64 = 10;
/// CPU seconds after which a build's weight has halved.
const HALF_CPU_SECS: f64 = 60.0;
const UPDATE_EVERY: Duration = Duration::from_millis(500);
const MOVE_TIMEOUT: Duration = Duration::from_millis(500);

pub fn enabled() -> bool {
    !matches!(
        std::env::var("JUSTRUST_SCHED").as_deref(),
        Ok("0") | Ok("off") | Ok("false")
    )
}

/// Weight for a build that has used `cpu_secs` of CPU so far.
pub fn weight_for(cpu_secs: f64) -> u64 {
    let w = MAX_WEIGHT as f64 * HALF_CPU_SECS / (HALF_CPU_SECS + cpu_secs.max(0.0));
    (w.round() as u64).clamp(MIN_WEIGHT, MAX_WEIGHT)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default, PartialEq)]
pub struct SchedInfo {
    pub unit: String,
    /// Lowest weight the build reached (its weight when it finished).
    pub final_weight: u64,
    /// Seconds spent creating the scope before cargo started.
    pub setup_secs: f64,
}

/// This process's scope. Dropping it stops the weight updates.
pub struct Scope {
    unit: String,
    setup_secs: f64,
    stop: Arc<AtomicBool>,
    min_weight: Arc<AtomicU64>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Scope {
    pub fn finish(mut self) -> SchedInfo {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        SchedInfo {
            unit: self.unit.clone(),
            final_weight: self.min_weight.load(Ordering::Relaxed),
            setup_secs: self.setup_secs,
        }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

fn cgroup_of(pid: u32) -> Option<String> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()?;
    parse_cgroup(&s)
}

fn parse_cgroup(s: &str) -> Option<String> {
    s.lines()
        .find_map(|l| l.strip_prefix("0::"))
        .map(|p| p.trim().to_owned())
}

fn parse_usage_secs(cpu_stat: &str) -> Option<f64> {
    cpu_stat
        .lines()
        .find_map(|l| l.strip_prefix("usage_usec "))
        .and_then(|v| v.trim().parse::<u64>().ok())
        .map(|us| us as f64 / 1e6)
}

/// Move this process into a new scope (call before spawning cargo).
pub fn enter(run_id: &str) -> Option<Scope> {
    if !enabled() || std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_none() {
        return None;
    }
    let t0 = Instant::now();
    let pid = std::process::id();
    // Already scheduled by an outer justrust (nested cargo): keep that scope.
    if cgroup_of(pid).is_some_and(|c| c.contains(&format!("/{SLICE}/"))) {
        return None;
    }
    let unit = format!("justrust-{run_id}.scope");
    let ok = Command::new("busctl")
        .args([
            "--user",
            "call",
            "org.freedesktop.systemd1",
            "/org/freedesktop/systemd1",
            "org.freedesktop.systemd1.Manager",
            "StartTransientUnit",
            "ssa(sv)a(sa(sv))",
            &unit,
            "fail",
            "4",
            "PIDs",
            "au",
            "1",
            &pid.to_string(),
            "Slice",
            "s",
            SLICE,
            "CPUWeight",
            "t",
            &MAX_WEIGHT.to_string(),
            "CollectMode",
            "s",
            "inactive-or-failed",
            "0",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success();
    if !ok {
        return None;
    }
    // The job is asynchronous: wait until this process really moved, so
    // cargo starts inside the scope.
    let deadline = Instant::now() + MOVE_TIMEOUT;
    let cg = loop {
        match cgroup_of(pid) {
            Some(cg) if cg.ends_with(&unit) => break cg,
            Some(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(2)),
            _ => return None,
        }
    };
    let cgroup = PathBuf::from(format!("/sys/fs/cgroup{cg}"));
    let stop = Arc::new(AtomicBool::new(false));
    let min_weight = Arc::new(AtomicU64::new(MAX_WEIGHT));
    let thread = {
        let (stop, min) = (stop.clone(), min_weight.clone());
        std::thread::spawn(move || {
            let mut last = MAX_WEIGHT;
            while !stop.load(Ordering::SeqCst) {
                std::thread::sleep(UPDATE_EVERY);
                let Some(used) = std::fs::read_to_string(cgroup.join("cpu.stat"))
                    .ok()
                    .as_deref()
                    .and_then(parse_usage_secs)
                else {
                    break;
                };
                let w = weight_for(used);
                if w != last && std::fs::write(cgroup.join("cpu.weight"), w.to_string()).is_ok() {
                    last = w;
                    min.fetch_min(w, Ordering::Relaxed);
                }
            }
        })
    };
    Some(Scope {
        unit,
        setup_secs: t0.elapsed().as_secs_f64(),
        stop,
        min_weight,
        thread: Some(thread),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_decays_with_attained_cpu() {
        assert_eq!(weight_for(0.0), 1000);
        assert_eq!(weight_for(60.0), 500);
        assert_eq!(weight_for(120.0), 333);
        assert!(weight_for(1200.0) < 60);
        assert_eq!(weight_for(1e9), MIN_WEIGHT);
        assert_eq!(weight_for(-5.0), 1000);
    }

    #[test]
    fn parses_cgroup_and_usage() {
        assert_eq!(
            parse_cgroup("0::/user.slice/app.slice/x.scope\n").as_deref(),
            Some("/user.slice/app.slice/x.scope")
        );
        assert_eq!(parse_cgroup("1:name=systemd:/x\n"), None);
        assert_eq!(
            parse_usage_secs("usage_usec 2500000\nuser_usec 1\n"),
            Some(2.5)
        );
        assert_eq!(parse_usage_secs("user_usec 1\n"), None);
    }

    #[test]
    fn disabled_by_env_never_moves() {
        // SAFETY: test-local env change; no other test reads JUSTRUST_SCHED.
        unsafe { std::env::set_var("JUSTRUST_SCHED", "0") };
        assert!(enter("test-disabled").is_none());
        unsafe { std::env::remove_var("JUSTRUST_SCHED") };
    }
}
