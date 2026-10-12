//! Process and machine readers for build-wide resource sampling: `/proc` on
//! Linux, libproc, Mach, and sysctl on macOS. One API for both, so the
//! sampler, `status`, and the reports never branch on the platform.
//!
//! Units are the platform's, read through the helpers here: process CPU is
//! in ticks of `clk_tck()` per second, resident memory in pages of
//! `page_size()`, and process start in ticks since boot. Anything a
//! platform cannot measure is `None`, never a zero that looks measured.

use serde::Serialize;

#[cfg(target_os = "linux")]
use linux as imp;
#[cfg(target_os = "macos")]
use macos as imp;

pub use imp::{
    all_pids, boot_time, cgroup, clk_tck, close_inherited_fds, cmdline, cpu_times, meminfo,
    pressure, read_stat,
};

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

/// This machine's name (`gethostname`), empty if it cannot be read.
pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is writable for its stated length.
    let r = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if r != 0 {
        return String::new();
    }
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

/// Aggregate CPU time, busy and total, in arbitrary but consistent units:
/// only the ratio of two samples' differences is meaningful.
#[derive(Debug, Clone, Copy, Default)]
pub struct CpuTimes {
    pub busy: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct MemInfo {
    pub total_mb: u64,
    pub available_mb: u64,
}

/// Cumulative pressure-stall microseconds ("some" line) for cpu, memory, io.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Pressure {
    pub cpu_us: u64,
    pub memory_us: u64,
    pub io_us: u64,
}

#[derive(Debug, Clone)]
pub struct ProcStat {
    pub pid: i32,
    pub ppid: i32,
    pub comm: String,
    /// User plus system CPU in `clk_tck()` ticks.
    pub cpu_ticks: u64,
    pub rss_pages: u64,
    /// Process start in ticks since boot, to tell reused pids apart.
    pub start_ticks: u64,
}

#[cfg(target_os = "linux")]
mod linux {
    use super::{CpuTimes, MemInfo, Pressure, ProcStat};

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

    fn psi_some_total(kind: &str) -> Option<u64> {
        let s = std::fs::read_to_string(format!("/proc/pressure/{kind}")).ok()?;
        s.lines()
            .find(|l| l.starts_with("some"))
            .and_then(|l| l.split_whitespace().find_map(|f| f.strip_prefix("total=")))
            .and_then(|v| v.parse().ok())
    }

    /// `None` when the kernel has no pressure stall info (`CONFIG_PSI` off,
    /// `psi=0`, some containers), so reports say so instead of "0s".
    pub fn pressure() -> Option<Pressure> {
        Some(Pressure {
            cpu_us: psi_some_total("cpu")?,
            memory_us: psi_some_total("memory").unwrap_or(0),
            io_us: psi_some_total("io").unwrap_or(0),
        })
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
            .map(|b| super::join_args(&b))
            .unwrap_or_default()
    }
}

#[cfg(target_os = "macos")]
mod macos {
    //! libproc for processes, Mach host statistics for CPU and memory, and
    //! sysctl for boot time and argument vectors. No pressure stall info
    //! exists on macOS: `pressure()` is `None` and reports say so.
    #![allow(deprecated)] // libc points Mach calls at the mach2 crate; same ABI.

    use super::{CpuTimes, MemInfo, Pressure, ProcStat};

    /// Process CPU and start times are kept in nanoseconds.
    const NS: f64 = 1e9;

    /// Make a long-lived child not inherit the parent's open descriptors past
    /// stdio (see the Linux version). macOS has no `close_range`: mark every
    /// fd above stdio close-on-exec instead, which `fcntl` does
    /// async-signal-safely and which keeps std's exec-error pipe working.
    /// Only up to the highest fd open now (listed here, before the fork):
    /// looping over the whole fd limit after fork kept children alive long
    /// enough to hold other threads' flocks. Fds opened later by std are
    /// close-on-exec already.
    pub fn close_inherited_fds(cmd: &mut std::process::Command) {
        use std::os::unix::process::CommandExt;
        let max = std::fs::read_dir("/dev/fd")
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| e.file_name().to_str()?.parse::<i32>().ok())
                    .max()
                    .unwrap_or(1024)
            })
            .unwrap_or(1024)
            + 64;
        // SAFETY: fcntl is async-signal-safe; a closed fd only makes it fail
        // with EBADF.
        unsafe {
            cmd.pre_exec(move || {
                for fd in 3..=max {
                    libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
                }
                Ok(())
            });
        }
    }

    pub fn clk_tck() -> f64 {
        NS
    }

    /// No cgroups on macOS.
    pub fn cgroup(_pid: u32) -> Option<String> {
        None
    }

    fn sysctl<T>(mib: &mut [libc::c_int]) -> Option<T> {
        let mut v: T = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<T>();
        // SAFETY: the out buffer is a T and len its size.
        let r = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                mib.len() as u32,
                (&mut v as *mut T).cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        (r == 0 && len == std::mem::size_of::<T>()).then_some(v)
    }

    /// System boot time in seconds since the epoch (`kern.boottime`).
    pub fn boot_time() -> Option<f64> {
        let tv: libc::timeval = sysctl(&mut [libc::CTL_KERN, libc::KERN_BOOTTIME])?;
        (tv.tv_sec > 0).then(|| tv.tv_sec as f64 + tv.tv_usec as f64 / 1e6)
    }

    /// Machine-wide CPU ticks from `HOST_CPU_LOAD_INFO`.
    pub fn cpu_times() -> Option<CpuTimes> {
        let mut info: libc::host_cpu_load_info = unsafe { std::mem::zeroed() };
        let mut count = libc::HOST_CPU_LOAD_INFO_COUNT;
        // SAFETY: the out buffer holds HOST_CPU_LOAD_INFO_COUNT integers.
        let r = unsafe {
            libc::host_statistics(
                libc::mach_host_self(),
                libc::HOST_CPU_LOAD_INFO,
                (&mut info as *mut libc::host_cpu_load_info).cast(),
                &mut count,
            )
        };
        if r != libc::KERN_SUCCESS {
            return None;
        }
        let t = info.cpu_ticks.map(u64::from);
        let idle = t[libc::CPU_STATE_IDLE as usize];
        let total: u64 = t.iter().sum();
        Some(CpuTimes {
            busy: total - idle,
            total,
        })
    }

    /// Total from `hw.memsize`. Available is what Activity Monitor calls
    /// free: total minus app (anonymous, not purgeable), wired, and
    /// compressed memory. File cache and purgeable pages count as available,
    /// like Linux's MemAvailable.
    pub fn meminfo() -> Option<MemInfo> {
        let total: u64 = sysctl(&mut [libc::CTL_HW, libc::HW_MEMSIZE])?;
        let mut vm: libc::vm_statistics64 = unsafe { std::mem::zeroed() };
        let mut count = (std::mem::size_of::<libc::vm_statistics64>()
            / std::mem::size_of::<libc::integer_t>()) as u32;
        // SAFETY: the out buffer is a vm_statistics64 and count its size in
        // integers.
        let r = unsafe {
            libc::host_statistics64(
                libc::mach_host_self(),
                libc::HOST_VM_INFO64,
                (&mut vm as *mut libc::vm_statistics64).cast(),
                &mut count,
            )
        };
        let page = super::page_size();
        let available = if r == libc::KERN_SUCCESS {
            let used_pages = u64::from(vm.internal_page_count)
                .saturating_sub(u64::from(vm.purgeable_count))
                + u64::from(vm.wire_count)
                + u64::from(vm.compressor_page_count);
            total.saturating_sub(used_pages * page)
        } else {
            0
        };
        Some(MemInfo {
            total_mb: total >> 20,
            available_mb: available >> 20,
        })
    }

    /// macOS has no pressure stall accounting.
    pub fn pressure() -> Option<Pressure> {
        None
    }

    fn cstr(b: &[libc::c_char]) -> String {
        let b: Vec<u8> = b
            .iter()
            .take_while(|c| **c != 0)
            .map(|c| *c as u8)
            .collect();
        String::from_utf8_lossy(&b).into_owned()
    }

    fn timebase() -> f64 {
        static T: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
        *T.get_or_init(|| {
            let mut tb = libc::mach_timebase_info { numer: 0, denom: 0 };
            // SAFETY: writes the struct it is given.
            let r = unsafe { libc::mach_timebase_info(&mut tb) };
            if r == 0 && tb.denom > 0 {
                tb.numer as f64 / tb.denom as f64
            } else {
                1.0
            }
        })
    }

    /// One process, from `PROC_PIDTASKALLINFO`. Only this user's processes
    /// are readable, which covers the build tree.
    pub fn read_stat(pid: i32) -> Option<ProcStat> {
        let mut info: libc::proc_taskallinfo = unsafe { std::mem::zeroed() };
        let size = std::mem::size_of::<libc::proc_taskallinfo>() as i32;
        // SAFETY: the out buffer is a proc_taskallinfo of `size` bytes.
        let n = unsafe {
            libc::proc_pidinfo(
                pid,
                libc::PROC_PIDTASKALLINFO,
                0,
                (&mut info as *mut libc::proc_taskallinfo).cast(),
                size,
            )
        };
        if n != size {
            return None;
        }
        let b = &info.pbsd;
        let t = &info.ptinfo;
        // pbi_comm is cut at 16 bytes; pbi_name holds up to 32.
        let comm = match cstr(&b.pbi_name) {
            n if n.is_empty() => cstr(&b.pbi_comm),
            n => n,
        };
        // Task times are Mach absolute time units, not nanoseconds, on
        // Apple silicon.
        let cpu_ns = (t.pti_total_user + t.pti_total_system) as f64 * timebase();
        let started = b.pbi_start_tvsec as f64 + b.pbi_start_tvusec as f64 / 1e6;
        let since_boot = (started - boot_time().unwrap_or(0.0)).max(0.0);
        Some(ProcStat {
            pid,
            ppid: b.pbi_ppid as i32,
            comm,
            cpu_ticks: cpu_ns as u64,
            rss_pages: t.pti_resident_size / super::page_size(),
            start_ticks: (since_boot * NS) as u64,
        })
    }

    pub fn all_pids() -> Vec<i32> {
        // SAFETY: a null buffer asks for the count.
        let n = unsafe { libc::proc_listallpids(std::ptr::null_mut(), 0) };
        if n <= 0 {
            return Vec::new();
        }
        // Room for processes started between the two calls.
        let mut pids = vec![0i32; n as usize + 64];
        let bytes = (pids.len() * std::mem::size_of::<i32>()) as i32;
        // SAFETY: the buffer holds `bytes` bytes of pids.
        let n = unsafe { libc::proc_listallpids(pids.as_mut_ptr().cast(), bytes) };
        pids.truncate(n.max(0) as usize);
        pids.retain(|p| *p > 0);
        pids
    }

    /// Arguments from `kern.procargs2`: argc, the executable path, padding,
    /// then the NUL-separated argv (environment after it is ignored).
    pub fn cmdline(pid: i32) -> String {
        let argmax: libc::c_int =
            sysctl(&mut [libc::CTL_KERN, libc::KERN_ARGMAX]).unwrap_or(1 << 18);
        let mut buf = vec![0u8; argmax.max(4096) as usize];
        let mut len = buf.len();
        let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
        // SAFETY: the buffer is writable for `len` bytes.
        let r = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            )
        };
        if r != 0 || len < 4 {
            return String::new();
        }
        parse_procargs(&buf[..len])
    }

    pub(super) fn parse_procargs(b: &[u8]) -> String {
        let Some(argc) = b
            .get(..4)
            .map(|a| i32::from_ne_bytes(a.try_into().unwrap()))
        else {
            return String::new();
        };
        let rest = &b[4..];
        // Skip the executable path and the NUL padding after it.
        let Some(end) = rest.iter().position(|c| *c == 0) else {
            return String::new();
        };
        let start = rest[end..]
            .iter()
            .position(|c| *c != 0)
            .map_or(rest.len(), |p| end + p);
        let mut args = Vec::new();
        let mut it = rest[start..].split(|c| *c == 0);
        for _ in 0..argc.max(0) {
            match it.next() {
                Some(a) => args.push(String::from_utf8_lossy(a).into_owned()),
                None => break,
            }
        }
        args.retain(|a| !a.is_empty());
        args.join(" ")
    }
}

/// NUL-separated arguments as one space-separated line.
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn join_args(b: &[u8]) -> String {
    b.split(|c| *c == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
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
        let spawn_sleeper = |p: &std::path::Path, close: bool| {
            use std::os::unix::process::CommandExt;
            let f = std::fs::File::create(p).unwrap();
            // A concurrent test's fork may share an earlier descriptor of
            // this file for a moment (macOS forks for pre_exec spawns).
            assert!((0..100).any(|_| {
                let ok = lock(&f);
                if !ok {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                ok
            }));
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
        let mut leaky = spawn_sleeper(&p, false);
        assert!(
            !lock(&std::fs::File::open(&p).unwrap()),
            "control: plain spawn keeps the lock"
        );
        leaky.kill().unwrap();
        leaky.wait().unwrap();
        // Its own file: the killed control child's lock cannot interfere.
        let p = p.with_extension("clean");
        let mut clean = spawn_sleeper(&p, true);
        // Another test thread that forks shares our descriptor until its
        // child execs, so the lock can look held for a moment. A leak into
        // the sleeper holds it for its whole 5 s: poll for up to 1 s.
        let freed = (0..100).any(|_| {
            let ok = lock(&std::fs::File::open(&p).unwrap());
            if !ok {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            ok
        });
        assert!(freed, "lock leaked into the daemon");
        clean.kill().unwrap();
        clean.wait().unwrap();
        let _ = std::fs::remove_file(&p);
        let _ = std::fs::remove_file(p.with_extension(""));
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
        let m = meminfo().expect("mem");
        assert!(m.total_mb > 0 && m.available_mb > 0 && m.available_mb <= m.total_mb);
        assert!(boot_time().is_some_and(|b| b > 1e9));
        assert!(!hostname().is_empty());
    }

    /// A child's tree link, CPU, start, and argv are read like `record`
    /// and `status` use them.
    #[test]
    fn reads_child_process() {
        // `exec` keeps the pid while the arguments become sleep's own.
        let mut c = std::process::Command::new("sh")
            .args(["-c", "exec sleep 5", "justrust-probe"])
            .spawn()
            .unwrap();
        let pid = c.id() as i32;
        let st = (0..200)
            .find_map(|_| {
                let s = read_stat(pid).filter(|s| s.comm == "sleep");
                if s.is_none() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                s
            })
            .expect("child stat");
        assert_eq!(st.ppid, std::process::id() as i32);
        assert!(all_pids().contains(&pid));
        assert_eq!(cmdline(pid), "sleep 5");
        let me = read_stat(std::process::id() as i32).unwrap();
        assert!(st.start_ticks >= me.start_ticks);
        let started = boot_time().unwrap() + st.start_ticks as f64 / clk_tck();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        assert!((now - started).abs() < 60.0, "start {started} vs now {now}");
        c.kill().unwrap();
        c.wait().unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn parses_procargs2() {
        let mut b = 3i32.to_ne_bytes().to_vec();
        b.extend_from_slice(b"/bin/sleep\0\0\0\0sleep\x005\0x y\0HOME=/h\0");
        assert_eq!(macos::parse_procargs(&b), "sleep 5 x y");
        assert_eq!(macos::parse_procargs(&[1, 0]), "");
    }
}
