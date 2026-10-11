//! Minimal /proc readers for build-wide resource sampling (Linux only).

use serde::Serialize;

/// Make a long-lived child not inherit the parent's open descriptors past
/// stdio. A daemon started from inside `flock file justrust ...` otherwise
/// keeps the flock's fd, and with it the lock, for hours after the build
/// that started it exited (a bench lock was held by `justrust __daemon`).
pub fn close_inherited_fds(cmd: &mut std::process::Command) {
    use std::os::unix::process::CommandExt;
    // SAFETY: close_range is async-signal-safe and touches only fds >= 3,
    // which the child does not use (stdio is set up before pre_exec runs).
    unsafe {
        cmd.pre_exec(|| {
            libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32);
            Ok(())
        });
    }
}

pub fn clk_tck() -> f64 {
    // SAFETY: sysconf has no preconditions.
    let v = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    if v > 0 { v as f64 } else { 100.0 }
}

pub fn page_size() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let v = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if v > 0 { v as u64 } else { 4096 }
}

pub fn ncpu() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Contents of /proc/<pid>/cgroup.
pub fn cgroup(pid: u32) -> Option<String> {
    std::fs::read_to_string(format!("/proc/{pid}/cgroup")).ok()
}

/// System boot time in seconds since the epoch (`btime` in /proc/stat).
pub fn boot_time() -> Option<f64> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    s.lines()
        .find_map(|l| l.strip_prefix("btime ")?.trim().parse::<f64>().ok())
}

/// Aggregate CPU jiffies from the first line of /proc/stat.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

pub fn cpu_times() -> Option<CpuTimes> {
    let s = std::fs::read_to_string("/proc/stat").ok()?;
    let line = s.lines().next()?;
    let v: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .filter_map(|x| x.parse().ok())
        .collect();
    if v.len() < 5 {
        return None;
    }
    let idle = v[3] + v.get(4).copied().unwrap_or(0);
    // Fields 8 and 9 (guest, guest_nice) are already counted in user and nice.
    let total: u64 = v.iter().take(8).sum();
    Some(CpuTimes {
        busy: total - idle,
        total,
    })
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemInfo {
    pub total_mb: u64,
    pub available_mb: u64,
}

pub fn meminfo() -> Option<MemInfo> {
    let s = std::fs::read_to_string("/proc/meminfo").ok()?;
    let mut m = MemInfo::default();
    for line in s.lines() {
        let mut it = line.split_whitespace();
        let (Some(k), Some(v)) = (it.next(), it.next()) else {
            continue;
        };
        let kb: u64 = v.parse().unwrap_or(0);
        match k {
            "MemTotal:" => m.total_mb = kb / 1024,
            "MemAvailable:" => m.available_mb = kb / 1024,
            _ => {}
        }
    }
    Some(m)
}

/// Cumulative pressure-stall microseconds ("some" line) for cpu, memory, io.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Pressure {
    pub cpu_us: u64,
    pub memory_us: u64,
    pub io_us: u64,
}

fn psi_some_total(kind: &str) -> u64 {
    let Ok(s) = std::fs::read_to_string(format!("/proc/pressure/{kind}")) else {
        return 0;
    };
    s.lines()
        .find(|l| l.starts_with("some"))
        .and_then(|l| l.split_whitespace().find_map(|f| f.strip_prefix("total=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or(0)
}

pub fn pressure() -> Pressure {
    Pressure {
        cpu_us: psi_some_total("cpu"),
        memory_us: psi_some_total("memory"),
        io_us: psi_some_total("io"),
    }
}

#[derive(Debug, Clone)]
pub struct ProcStat {
    pub pid: i32,
    pub ppid: i32,
    pub comm: String,
    /// utime + stime in clock ticks.
    pub cpu_ticks: u64,
    pub rss_pages: u64,
    /// Process start time in clock ticks since boot, to tell reused pids apart.
    pub start_ticks: u64,
}

pub fn read_stat(pid: i32) -> Option<ProcStat> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // comm is in parentheses and may contain spaces, so split at the last ')'.
    let open = s.find('(')?;
    let close = s.rfind(')')?;
    let comm = s[open + 1..close].to_owned();
    let rest: Vec<&str> = s[close + 2..].split_whitespace().collect();
    // rest[0] is field 3 (state).
    let field = |n: usize| {
        rest.get(n - 3)
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    };
    Some(ProcStat {
        pid,
        ppid: field(4) as i32,
        comm,
        cpu_ticks: field(14) + field(15),
        start_ticks: field(22),
        rss_pages: field(24),
    })
}

pub fn all_pids() -> Vec<i32> {
    let Ok(rd) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    rd.filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}

pub fn cmdline(pid: i32) -> String {
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .map(|b| {
            b.split(|c| *c == 0)
                .filter(|p| !p.is_empty())
                .map(|p| String::from_utf8_lossy(p).into_owned())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A child spawned through `close_inherited_fds` does not keep a flock
    /// its parent held (the bench-lock leak through `justrust __daemon`).
    #[test]
    fn daemon_child_does_not_keep_inherited_flock() {
        use std::os::fd::AsRawFd;
        let p = std::env::temp_dir().join(format!("jr-fdleak-{}", std::process::id()));
        let lock = |f: &std::fs::File| unsafe {
            libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0
        };
        let spawn_sleeper = |close: bool| {
            use std::os::unix::process::CommandExt;
            let f = std::fs::File::create(&p).unwrap();
            assert!(lock(&f));
            let fd = f.as_raw_fd();
            let mut c = std::process::Command::new("sleep");
            c.arg("5");
            // Inheritable in this child only, like the fd `flock(1)` hands
            // its command (dup2 clears CLOEXEC on the copy). The parent's fd
            // stays CLOEXEC, so other tests' children never get it.
            unsafe {
                c.pre_exec(move || {
                    libc::dup2(fd, 50);
                    Ok(())
                });
            }
            if close {
                close_inherited_fds(&mut c);
            }
            let child = c.spawn().unwrap();
            drop(f);
            child
        };
        let mut leaky = spawn_sleeper(false);
        assert!(
            !lock(&std::fs::File::open(&p).unwrap()),
            "control: plain spawn keeps the lock"
        );
        leaky.kill().unwrap();
        leaky.wait().unwrap();
        let mut clean = spawn_sleeper(true);
        assert!(
            lock(&std::fs::File::open(&p).unwrap()),
            "lock leaked into the daemon"
        );
        clean.kill().unwrap();
        clean.wait().unwrap();
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn reads_own_process() {
        let me = read_stat(std::process::id() as i32).expect("self stat");
        assert!(me.rss_pages > 0);
        assert!(!me.comm.is_empty());
    }

    #[test]
    fn reads_system_counters() {
        let c = cpu_times().expect("cpu");
        assert!(c.total >= c.busy);
        assert!(meminfo().expect("mem").total_mb > 0);
    }
}
