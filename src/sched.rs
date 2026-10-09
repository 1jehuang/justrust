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
//! sched_ext schedulers (this machine runs `scx_lavd`) ignore cgroup
//! weights, `nice`, and `cpu.max`; measured, a weight-1000 probe got the same
//! CPU as a weight-10 one. They do honor CPU affinity. So, second mechanism:
//! while another justrust build is young and running, a build that has used
//! more than `YOUNG_CPU_SECS` is pinned to the slower half of the CPUs, which
//! leaves the fastest cores to the edit loop. It gets every CPU back as soon
//! as no young build is running. FINDINGS.md section 13 has the numbers.
//!
//! All builds share the one slice, so together they compete with the rest of
//! the desktop like a single app.
//!
//! Fails open: no systemd user bus, no `busctl`, a slow move, or
//! `JUSTRUST_SCHED=0` just run cargo where justrust already runs.

use std::collections::HashMap;
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
/// A build that has used less CPU than this is "young": an edit loop on
/// Jcode Desktop costs 10-120 CPU-s, a cold build 1,000+.
const YOUNG_CPU_SECS: f64 = 200.0;
/// CPU a young build must use per check (0.5 s) to count as running.
const ACTIVE_CPU_SECS: f64 = 0.1;

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
    /// Seconds this (old) build spent off the fastest cores for younger ones.
    #[serde(default)]
    pub yielded_secs: f64,
}

/// This process's scope. Dropping it stops the weight updates.
pub struct Scope {
    unit: String,
    setup_secs: f64,
    stop: Arc<AtomicBool>,
    min_weight: Arc<AtomicU64>,
    yielded_ms: Arc<AtomicU64>,
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
            yielded_secs: self.yielded_ms.load(Ordering::Relaxed) as f64 / 1000.0,
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
    let yielded_ms = Arc::new(AtomicU64::new(0));
    let thread = {
        let (stop, min, yielded_ms) = (stop.clone(), min_weight.clone(), yielded_ms.clone());
        std::thread::spawn(move || {
            let cpus = Cpus::detect();
            let mut seen = HashMap::new();
            let mut last = MAX_WEIGHT;
            let mut confined = false;
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
                // Weights only work under the kernel's fair scheduler.
                // sched_ext schedulers (scx_lavd, ...) ignore cgroup weights
                // and nice but honor affinity, so old builds also step off
                // the fastest cores while a young one runs.
                let Some(cpus) = &cpus else { continue };
                let young_other = cgroup
                    .parent()
                    .map(|slice| young_siblings(slice, &cgroup, &mut seen))
                    .unwrap_or(0);
                let want = used >= YOUNG_CPU_SECS && young_other > 0;
                if want {
                    set_affinity(&cgroup, &cpus.old);
                    yielded_ms.fetch_add(UPDATE_EVERY.as_millis() as u64, Ordering::Relaxed);
                } else if confined {
                    set_affinity(&cgroup, &cpus.all);
                }
                confined = want;
            }
        })
    };
    Some(Scope {
        unit,
        setup_secs: t0.elapsed().as_secs_f64(),
        stop,
        min_weight,
        yielded_ms,
        thread: Some(thread),
    })
}

/// Online CPUs, and the subset old builds keep while young builds run.
struct Cpus {
    all: Vec<usize>,
    old: Vec<usize>,
}

impl Cpus {
    fn detect() -> Option<Cpus> {
        let all = parse_cpu_list(
            std::fs::read_to_string("/sys/devices/system/cpu/online")
                .ok()?
                .trim(),
        );
        let freq = |c: usize| {
            std::fs::read_to_string(format!(
                "/sys/devices/system/cpu/cpu{c}/cpufreq/cpuinfo_max_freq"
            ))
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .unwrap_or(0)
        };
        let old = old_cpus(&all.iter().map(|&c| (c, freq(c))).collect::<Vec<_>>());
        (!old.is_empty()).then_some(Cpus { all, old })
    }
}

/// Reserve the fastest half of the CPUs (at least 2) for young builds.
fn old_cpus(cpus: &[(usize, u64)]) -> Vec<usize> {
    if cpus.len() < 4 {
        return Vec::new();
    }
    let mut by_speed = cpus.to_vec();
    // Fastest first; ties keep the lower numbers reserved.
    by_speed.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let reserve = (cpus.len() / 2).max(2);
    let mut old: Vec<usize> = by_speed[reserve..].iter().map(|c| c.0).collect();
    old.sort_unstable();
    old
}

fn parse_cpu_list(s: &str) -> Vec<usize> {
    let mut out = Vec::new();
    for part in s.split(',').filter(|p| !p.is_empty()) {
        match part.split_once('-') {
            Some((a, b)) => {
                if let (Ok(a), Ok(b)) = (a.parse::<usize>(), b.parse::<usize>()) {
                    out.extend(a..=b);
                }
            }
            None => out.extend(part.parse::<usize>().ok()),
        }
    }
    out
}

/// Running justrust builds other than `me` that are still young and used CPU
/// since the last check (an idle `justrust run` server does not count).
/// `seen` keeps each sibling's previous usage between calls.
fn young_siblings(
    slice: &std::path::Path,
    me: &std::path::Path,
    seen: &mut HashMap<PathBuf, f64>,
) -> usize {
    let Ok(rd) = std::fs::read_dir(slice) else {
        return 0;
    };
    let mut now = HashMap::new();
    let mut n = 0;
    for p in rd.flatten().map(|e| e.path()) {
        if p == me || p.extension().is_none_or(|x| x != "scope") {
            continue;
        }
        if !std::fs::read_to_string(p.join("cgroup.procs")).is_ok_and(|s| !s.trim().is_empty()) {
            continue;
        }
        let Some(u) = std::fs::read_to_string(p.join("cpu.stat"))
            .ok()
            .as_deref()
            .and_then(parse_usage_secs)
        else {
            continue;
        };
        // New siblings count right away: they just started.
        let active = seen.get(&p).is_none_or(|&prev| u - prev >= ACTIVE_CPU_SECS);
        if u < YOUNG_CPU_SECS && active {
            n += 1;
        }
        now.insert(p, u);
    }
    *seen = now;
    n
}

/// Pin every thread in the cgroup (children inherit it from their parent).
fn set_affinity(cgroup: &std::path::Path, cpus: &[usize]) {
    let Ok(tids) = std::fs::read_to_string(cgroup.join("cgroup.threads")) else {
        return;
    };
    // SAFETY: cpu_set_t is plain data; CPU_SET bounds-checks the index.
    let set = unsafe {
        let mut set: libc::cpu_set_t = std::mem::zeroed();
        for &c in cpus {
            libc::CPU_SET(c, &mut set);
        }
        set
    };
    for tid in tids.lines().filter_map(|l| l.trim().parse::<i32>().ok()) {
        // SAFETY: plain syscall; a vanished tid just fails with ESRCH.
        unsafe {
            libc::sched_setaffinity(tid, std::mem::size_of::<libc::cpu_set_t>(), &set);
        }
    }
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
    fn reserves_the_fastest_half_for_young_builds() {
        // Like the XPS 14: 4 fast P cores, 8 E cores, 4 LP-E cores.
        let cpus: Vec<(usize, u64)> = (0..16)
            .map(|c| {
                (
                    c,
                    if c < 4 {
                        5_100
                    } else if c < 12 {
                        4_000
                    } else {
                        3_700
                    },
                )
            })
            .collect();
        assert_eq!(old_cpus(&cpus), vec![8, 9, 10, 11, 12, 13, 14, 15]);
        // Unknown frequencies: keep the high-numbered half.
        let flat: Vec<(usize, u64)> = (0..8).map(|c| (c, 0)).collect();
        assert_eq!(old_cpus(&flat), vec![4, 5, 6, 7]);
        assert!(old_cpus(&[(0, 1), (1, 1)]).is_empty());
    }

    #[test]
    fn parses_cpu_lists() {
        assert_eq!(parse_cpu_list("0-3,8,10-11"), vec![0, 1, 2, 3, 8, 10, 11]);
        assert_eq!(parse_cpu_list("0"), vec![0]);
        assert!(parse_cpu_list("").is_empty());
    }

    #[test]
    fn counts_only_young_running_siblings() {
        let dir = std::env::temp_dir().join(format!("jr-sched-test-{}", std::process::id()));
        let mk = |name: &str, procs: &str, usec: u64| {
            let p = dir.join(name);
            std::fs::create_dir_all(&p).unwrap();
            std::fs::write(p.join("cgroup.procs"), procs).unwrap();
            std::fs::write(p.join("cpu.stat"), format!("usage_usec {usec}\n")).unwrap();
            p
        };
        let me = mk("me.scope", "1\n", 0);
        let young = mk("young.scope", "2\n", 5_000_000);
        mk("old.scope", "3\n", 900_000_000);
        mk("done.scope", "", 1_000_000);
        mk("other.slice", "4\n", 0);
        let mut seen = HashMap::new();
        assert_eq!(young_siblings(&dir, &me, &mut seen), 1);
        // Idle since the last check: no longer counts.
        assert_eq!(young_siblings(&dir, &me, &mut seen), 0);
        std::fs::write(young.join("cpu.stat"), "usage_usec 6000000\n").unwrap();
        assert_eq!(young_siblings(&dir, &me, &mut seen), 1);
        std::fs::remove_dir_all(&dir).unwrap();
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
