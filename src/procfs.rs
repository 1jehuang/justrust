//! Process and machine readers for build-wide resource sampling: `/proc` on
//! Linux, libproc, Mach, and sysctl on macOS, Toolhelp and the process and
//! system information APIs on Windows. One API for all, so the sampler,
//! `status`, and the reports never branch on the platform.
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
#[cfg(windows)]
use windows as imp;

pub use imp::{
    all_pids, boot_time, cgroup, clk_tck, close_inherited_fds, cmdline, cpu_times, meminfo,
    pressure, read_stat,
};

/// Installed memory in MiB, for sizing builds: MemTotal, or more if a
/// balloon is currently holding part of it (Linux memory blocks).
pub fn installed_mb() -> Option<u64> {
    let total = meminfo()?.total_mb;
    #[cfg(target_os = "linux")]
    let total = total.max(linux::online_block_mb().unwrap_or(0));
    (total > 0).then_some(total)
}

#[cfg(unix)]
pub fn page_size() -> u64 {
    // SAFETY: sysconf has no preconditions.
    let v = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    if v > 0 { v as u64 } else { 4096 }
}

/// Windows reports working sets in bytes: `read_stat` divides by this.
#[cfg(windows)]
pub fn page_size() -> u64 {
    4096
}

/// A pathname for synchronous helpers (hashing, patching and uploading) to
/// read our loaded image, even after its original directory entry is removed.
/// Use our PID, not `self`: a child must not hash its own executable instead.
/// Do not persist this path beyond the lifetime of the calling process.
#[cfg(target_os = "linux")]
pub fn running_executable_path() -> std::io::Result<std::path::PathBuf> {
    Ok(format!("/proc/{}/exe", std::process::id()).into())
}

#[cfg(not(target_os = "linux"))]
pub fn running_executable_path() -> std::io::Result<std::path::PathBuf> {
    std::env::current_exe()
}

/// Open the running executable itself, including after an atomic installer
/// replacement removed its original pathname. Keep procfs details here.
#[cfg(unix)]
pub fn running_executable() -> std::io::Result<std::fs::File> {
    #[cfg(target_os = "linux")]
    let path = std::path::PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let path = std::env::current_exe()?;
    std::fs::File::open(path)
}

/// The account's own non-dumpable session processes, which can never be
/// building inside a mirror: this process's ancestors (sshd for the agent),
/// and the per-user `systemd --user` manager and its `(sd-pam)` helper.
#[cfg(target_os = "linux")]
fn is_session_plumbing(base: &std::path::Path, pid: &str) -> bool {
    let Ok(pid_num) = pid.parse::<u32>() else {
        return false;
    };
    // Only processes the kernel hides from their owner: a readable cwd is
    // inspected normally and never exempt.
    if std::fs::read_link(base.join("cwd")).is_ok() {
        return false;
    }
    if ancestors_of_self().contains(&pid_num) {
        return true;
    }
    let cmdline = std::fs::read(base.join("cmdline")).unwrap_or_default();
    let argv: Vec<&[u8]> = cmdline
        .split(|b| *b == 0)
        .filter(|a| !a.is_empty())
        .collect();
    is_user_manager_argv(&argv)
}

/// `systemd --user` itself, its PAM helper `(sd-pam)`, or an sshd session
/// process (`sshd: user@notty`) of another connection, e.g. the persistent
/// agent session. Those set their own title and never run builds.
#[cfg(target_os = "linux")]
fn is_user_manager_argv(argv: &[&[u8]]) -> bool {
    match argv {
        [b"(sd-pam)"] => true,
        [title] if title.starts_with(b"sshd: ") && title.contains(&b'@') => true,
        [exe, flag] => *flag == b"--user" && exe.ends_with(b"/systemd"),
        _ => false,
    }
}

/// PIDs of this process's parent chain (excluding itself).
#[cfg(target_os = "linux")]
fn ancestors_of_self() -> Vec<u32> {
    ancestors_of(std::process::id())
}

/// PIDs of `pid`'s parent chain (excluding itself).
#[cfg(target_os = "linux")]
fn ancestors_of(mut pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    for _ in 0..64 {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            break;
        };
        // The comm field may contain spaces and parens: fields after the last ')'.
        let Some(after) = stat.rsplit_once(')').map(|(_, rest)| rest) else {
            break;
        };
        let Some(ppid) = after.split_whitespace().nth(1).and_then(|p| p.parse().ok()) else {
            break;
        };
        if ppid <= 1 {
            break;
        }
        out.push(ppid);
        pid = ppid;
    }
    out
}

/// Holders of the file locks that processes matching `waits` are blocked on,
/// from `/proc/locks`: a blocked request is a `N: -> ...` line under the lock
/// `N:` it waits for. Field 5 (4 after the arrow) is the pid.
pub fn parse_lock_holders(locks: &str, waits: impl Fn(u32) -> bool) -> Vec<u32> {
    let mut held: std::collections::HashMap<&str, u32> = Default::default();
    let mut out = Vec::new();
    for line in locks.lines() {
        let mut f = line.split_whitespace();
        let Some(id) = f.next() else { continue };
        let rest: Vec<&str> = f.collect();
        let blocked = rest.first() == Some(&"->");
        let Some(pid) = rest
            .get(3 + blocked as usize)
            .and_then(|p| p.parse::<u32>().ok())
        else {
            continue;
        };
        if !blocked {
            held.insert(id, pid);
        } else if waits(pid)
            && let Some(&holder) = held.get(id)
            && !waits(holder)
            && !out.contains(&holder)
        {
            out.push(holder);
        }
    }
    out
}

/// Who holds a file lock that this process or one below it is blocked on,
/// one line each: `pid N (command line[, run ID])`. Empty when nothing is
/// blocked, or off Linux.
pub fn lock_holders_blocking_us() -> Vec<String> {
    #[cfg(target_os = "linux")]
    {
        let me = std::process::id();
        let Ok(locks) = std::fs::read_to_string("/proc/locks") else {
            return Vec::new();
        };
        parse_lock_holders(&locks, |p| p == me || ancestors_of(p).contains(&me))
            .into_iter()
            .map(|pid| {
                let argv = std::fs::read(format!("/proc/{pid}/cmdline"))
                    .map(|b| join_args(&b))
                    .unwrap_or_default();
                let run = std::fs::read(format!("/proc/{pid}/environ"))
                    .ok()
                    .and_then(|e| {
                        e.split(|&b| b == 0).find_map(|kv| {
                            kv.strip_prefix(b"JUSTRUST_RUN_ID_OVERRIDE=")
                                .map(|v| String::from_utf8_lossy(v).into_owned())
                        })
                    });
                let argv: String = argv.chars().take(160).collect();
                match run {
                    Some(r) => format!("pid {pid} ({argv}, run {r})"),
                    None => format!("pid {pid} ({argv})"),
                }
            })
            .collect()
    }
    #[cfg(not(target_os = "linux"))]
    Vec::new()
}

/// A running executable or an mmap remains live after its original file
/// descriptor closes. Retain outputs on any unreadable same-principal probe.
#[cfg(target_os = "linux")]
fn executable_or_mapping_in_use(
    base: &std::path::Path,
    path: &std::path::Path,
    same_principal: bool,
) -> bool {
    match std::fs::read_link(base.join("exe")) {
        Ok(exe) if exe.starts_with(path) => return true,
        Err(e) if same_principal && e.kind() != std::io::ErrorKind::NotFound => return true,
        _ => (),
    }
    match std::fs::read(base.join("maps")) {
        Ok(maps) => maps_in_use(&maps, path).unwrap_or(true),
        Err(e) => same_principal && e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// Parse kernel maps as bytes: pathname fields can contain spaces, non-UTF8,
/// and escaped newlines. Match path components, never string prefixes.
#[cfg(target_os = "linux")]
fn maps_in_use(maps: &[u8], path: &std::path::Path) -> Option<bool> {
    use std::os::unix::ffi::OsStringExt;
    for line in maps.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
        let mut rest = line;
        let mut inode = &b""[..];
        for index in 0..5 {
            let end = rest
                .iter()
                .position(u8::is_ascii_whitespace)
                .unwrap_or(rest.len());
            if end == 0 {
                return None;
            }
            if index == 4 {
                inode = &rest[..end];
            }
            rest = &rest[end..];
            while rest.first().is_some_and(u8::is_ascii_whitespace) {
                rest = &rest[1..];
            }
        }
        let inode = std::str::from_utf8(inode).ok()?.parse::<u64>().ok()?;
        if inode == 0 || rest.first() != Some(&b'/') {
            continue;
        }
        // The kernel escapes newline bytes but literal backslash-octal names
        // are ambiguous. Accept both spellings conservatively.
        if std::path::PathBuf::from(std::ffi::OsString::from_vec(rest.to_vec())).starts_with(path) {
            return Some(true);
        }
        let mut decoded = Vec::with_capacity(rest.len());
        let mut i = 0;
        while i < rest.len() {
            if rest[i] == b'\\'
                && rest
                    .get(i + 1..i + 4)
                    .is_some_and(|s| s.iter().all(|b| (b'0'..=b'7').contains(b)))
            {
                let value = u16::from(rest[i + 1] - b'0') * 64
                    + u16::from(rest[i + 2] - b'0') * 8
                    + u16::from(rest[i + 3] - b'0');
                decoded.push(u8::try_from(value).ok()?);
                i += 4;
            } else {
                decoded.push(rest[i]);
                i += 1;
            }
        }
        let mapped = std::path::PathBuf::from(std::ffi::OsString::from_vec(decoded));
        if mapped.starts_with(path) {
            return Some(true);
        }
    }
    Some(false)
}

/// Conservative live-use exclusion for native target GC. Non-Linux hosts
/// cannot prove inactivity here and must retain targets.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn path_in_use_by_other(path: &std::path::Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let me = std::process::id().to_string();
        let Ok(procs) = std::fs::read_dir("/proc") else {
            return true;
        };
        for entry in procs {
            let Ok(entry) = entry else {
                return true;
            };
            let name = entry.file_name();
            let Some(pid) = name
                .to_str()
                .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            else {
                continue;
            };
            if pid == me {
                continue;
            }
            let base = entry.path();
            // Inspect all readable processes. Inaccessible processes of this
            // principal are uncertainty, not proof of inactivity. Foreign
            // private /proc entries cannot disable the whole account's GC.
            use std::os::unix::fs::MetadataExt;
            // Pin this proc directory before reading its children. A recycled
            // numeric PID must not splice observations from two processes.
            // /proc/self/fd resolves through the held inode, not /proc/PID again.
            use std::os::fd::AsRawFd;
            let process = match std::fs::File::open(&base) {
                Ok(process) => process,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return true,
            };
            let base = std::path::PathBuf::from(format!("/proc/self/fd/{}", process.as_raw_fd()));
            let meta = match process.metadata() {
                Ok(meta) => meta,
                Err(_) => return true,
            };
            let same_principal = meta.uid() == unsafe { libc::geteuid() };
            // The account's own session plumbing is non-dumpable, so even its
            // owner gets EACCES on cwd/fd: the agent's ancestors (`sshd:
            // user@notty`, which starts it) and the user's `systemd --user`
            // manager with its `(sd-pam)` child. None of them can run a build
            // inside a mirror. Treating them as uncertainty made every root
            // permanently "in use" on hosted machines (2026-10-20: all 14
            // registered roots skipped as InUse at 99% disk). Every other
            // unreadable process of this principal still fails closed.
            let same_principal = same_principal && !is_session_plumbing(&base, pid);
            if executable_or_mapping_in_use(&base, path, same_principal) {
                return true;
            }
            match std::fs::read_link(base.join("cwd")) {
                Ok(cwd) if cwd.starts_with(path) => return true,
                Err(e) if same_principal && e.kind() != std::io::ErrorKind::NotFound => {
                    return true;
                }
                _ => (),
            }
            match std::fs::read_dir(base.join("fd")) {
                Ok(fds) => {
                    for fd in fds {
                        let Ok(fd) = fd else {
                            return true;
                        };
                        match std::fs::read_link(fd.path()) {
                            Ok(p) if p.starts_with(path) => return true,
                            Err(e)
                                if same_principal && e.kind() != std::io::ErrorKind::NotFound =>
                            {
                                return true;
                            }
                            _ => (),
                        }
                    }
                }
                Err(e) if same_principal && e.kind() != std::io::ErrorKind::NotFound => {
                    return true;
                }
                _ => (),
            }
        }
        false
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        true
    }
}

/// Read the caller's file creation mask without changing process-global state.
/// Linux exposes it in procfs. Other Unix callers use an isolated shell child.
pub fn current_umask() -> Option<u32> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let value = status
            .lines()
            .find_map(|line| line.strip_prefix("Umask:"))?;
        parse_umask(value.trim())
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let output = std::process::Command::new("sh")
            .args(["-c", "umask"])
            .env_remove("BASH_ENV")
            .env_remove("ENV")
            .output()
            .ok()?;
        output.status.success().then_some(())?;
        parse_umask(std::str::from_utf8(&output.stdout).ok()?.trim())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(unix)]
pub fn parse_umask(value: &str) -> Option<u32> {
    if value.is_empty() || value.len() > 4 || !value.bytes().all(|b| (b'0'..=b'7').contains(&b)) {
        return None;
    }
    u32::from_str_radix(value, 8)
        .ok()
        .filter(|mask| *mask <= 0o777)
}

pub fn ncpu() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// This machine's name (`gethostname`), empty if it cannot be read.
#[cfg(unix)]
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

/// This machine's NetBIOS name (`COMPUTERNAME`), empty if unset.
#[cfg(windows)]
pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .unwrap_or_default()
        .trim()
        .to_lowercase()
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
        // SAFETY: close_range is async-signal-safe. CLOSE_RANGE_CLOEXEC marks
        // fds >= 3 close-on-exec without closing Rust std's own CLOEXEC error
        // pipe before execve; if execve fails, std still reports the errno
        // cleanly instead of aborting on a closed error pipe. Fall back to
        // immediate close only if the kernel rejects CLOSE_RANGE_CLOEXEC (pre-5.11).
        const CLOSE_RANGE_CLOEXEC: u32 = 1 << 2;
        unsafe {
            use std::os::unix::process::CommandExt;
            cmd.pre_exec(|| {
                if libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, CLOSE_RANGE_CLOEXEC) != 0
                    && *libc::__errno_location() == libc::EINVAL
                {
                    libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32);
                }
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

    /// Installed memory from the online memory blocks. A virtio balloon
    /// lowers MemTotal: hosted inflates an idle slot's balloon to ~6 GiB and
    /// deflates it within seconds of new work, so a build sized from MemTotal
    /// at start got `CARGO_BUILD_JOBS=2` on a 96-CPU, 30-GiB guest (cold
    /// release jcode 584 s vs 214 s at 14 jobs). Blocks are not ballooned.
    pub(super) fn online_block_mb() -> Option<u64> {
        let dir = std::path::Path::new("/sys/devices/system/memory");
        let block = std::fs::read_to_string(dir.join("block_size_bytes")).ok()?;
        let states = std::fs::read_dir(dir).ok()?.flatten().filter_map(|e| {
            e.file_name()
                .to_str()?
                .starts_with("memory")
                .then(|| std::fs::read_to_string(e.path().join("state")).ok())?
        });
        online_mb(&block, states)
    }

    pub(super) fn online_mb(block_hex: &str, states: impl Iterator<Item = String>) -> Option<u64> {
        let block = u64::from_str_radix(block_hex.trim(), 16).ok()?;
        let online = states.filter(|s| s.trim() == "online").count() as u64;
        (online > 0).then(|| block.saturating_mul(online) >> 20)
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
        parse_stat(pid, &s)
    }

    fn parse_stat(pid: i32, s: &str) -> Option<ProcStat> {
        // comm may contain spaces and parentheses, so split at the last ')'.
        let open = s.find('(')?;
        let close = s.rfind(')')?;
        let comm = s.get(open + 1..close)?.to_owned();
        let mut rest = s.get(close + 2..)?.split_whitespace();
        // Consume only through RSS. nth skips relative to the previous field.
        let mut field = |skip| {
            rest.nth(skip)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(0)
        };
        Some(ProcStat {
            pid,
            ppid: field(1) as i32, // field 4
            comm,
            cpu_ticks: field(9) + field(0), // fields 14 and 15
            start_ticks: field(6),          // field 22
            rss_pages: field(1),            // field 24
        })
    }

    #[test]
    fn stat_parser_handles_names_missing_fields_and_reused_pids() {
        let line = "98 (a ) (b) S 4 5 6 7 8 9 10 11 12 13 bad 15 16 17 18 19 20 21 22 23 -4";
        let st = parse_stat(98, line).unwrap();
        assert_eq!(
            (
                st.comm.as_str(),
                st.ppid,
                st.cpu_ticks,
                st.start_ticks,
                st.rss_pages
            ),
            ("a ) (b", 4, 15, 22, 0)
        );
        let reused = parse_stat(98, &line.replace("22 23 -4", "99 23 24")).unwrap();
        assert_eq!(
            (reused.pid, reused.start_ticks, reused.rss_pages),
            (98, 99, 24)
        );
        let short = parse_stat(98, "98 (a) S 4").unwrap();
        assert_eq!(
            (
                short.ppid,
                short.cpu_ticks,
                short.start_ticks,
                short.rss_pages
            ),
            (4, 0, 0, 0)
        );
        for malformed in ["98 a", "98 )(", "98 (a)"] {
            assert!(parse_stat(98, malformed).is_none());
        }
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

#[cfg(windows)]
mod windows {
    //! Toolhelp snapshots for the process tree, `GetProcessTimes` and the
    //! working set for each process, `GetSystemTimes` and
    //! `GlobalMemoryStatusEx` for the machine. Times are FILETIME ticks
    //! (100 ns). No pressure stall info and no cgroups exist on Windows.

    use super::{CpuTimes, MemInfo, Pressure, ProcStat};
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows_sys::Win32::System::Threading::{
        GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    };

    /// FILETIME ticks per second.
    const TICKS: f64 = 1e7;
    /// FILETIME ticks between 1601-01-01 and the Unix epoch.
    const EPOCH_DIFF: u64 = 116_444_736_000_000_000;

    fn ft(t: FILETIME) -> u64 {
        (t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64
    }

    /// Nothing to do: std creates every handle non-inheritable, and a
    /// child gets only its stdio.
    pub fn close_inherited_fds(_cmd: &mut std::process::Command) {}

    pub fn clk_tck() -> f64 {
        TICKS
    }

    pub fn cgroup(_pid: u32) -> Option<String> {
        None
    }

    /// Now minus the milliseconds since boot.
    pub fn boot_time() -> Option<f64> {
        // SAFETY: no arguments.
        let up = unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() };
        let now = crate::paths::now();
        (now > 0.0).then(|| now - up as f64 / 1000.0)
    }

    /// Kernel time includes idle time.
    pub fn cpu_times() -> Option<CpuTimes> {
        let (mut idle, mut kernel, mut user) = Default::default();
        // SAFETY: three FILETIME out parameters.
        let ok = unsafe {
            windows_sys::Win32::System::Threading::GetSystemTimes(&mut idle, &mut kernel, &mut user)
        };
        if ok == 0 {
            return None;
        }
        let total = ft(kernel) + ft(user);
        Some(CpuTimes {
            busy: total.saturating_sub(ft(idle)),
            total,
        })
    }

    pub fn meminfo() -> Option<MemInfo> {
        use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        // SAFETY: the struct is plain data sized by dwLength.
        unsafe {
            let mut m: MEMORYSTATUSEX = std::mem::zeroed();
            m.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
            if GlobalMemoryStatusEx(&mut m) == 0 {
                return None;
            }
            Some(MemInfo {
                total_mb: m.ullTotalPhys >> 20,
                available_mb: m.ullAvailPhys >> 20,
            })
        }
    }

    pub fn pressure() -> Option<Pressure> {
        None
    }

    /// pid -> (parent pid, executable name without `.exe`).
    type Table = HashMap<i32, (i32, String)>;

    /// The sampler asks for every pid and then each one's stat: one
    /// snapshot serves a whole sample.
    static SNAPSHOT: Mutex<Option<(Instant, Table)>> = Mutex::new(None);
    const SNAPSHOT_TTL: Duration = Duration::from_millis(100);

    fn snapshot() -> Table {
        let mut t = Table::new();
        // SAFETY: the snapshot handle is checked and closed; the entry
        // struct is sized by dwSize.
        unsafe {
            let h = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if h == INVALID_HANDLE_VALUE || h.is_null() {
                return t;
            }
            let mut e: PROCESSENTRY32W = std::mem::zeroed();
            e.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(h, &mut e);
            while ok != 0 {
                let end = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(260);
                let name = String::from_utf16_lossy(&e.szExeFile[..end]);
                let name = crate::sys::program_name(name.as_ref());
                t.insert(e.th32ProcessID as i32, (e.th32ParentProcessID as i32, name));
                ok = Process32NextW(h, &mut e);
            }
            CloseHandle(h);
        }
        t
    }

    fn with_table<T>(fresh: bool, f: impl FnOnce(&Table) -> T) -> T {
        let mut g = SNAPSHOT.lock().unwrap_or_else(|e| e.into_inner());
        let stale = fresh || g.as_ref().is_none_or(|(at, _)| at.elapsed() > SNAPSHOT_TTL);
        if stale {
            *g = Some((Instant::now(), snapshot()));
        }
        f(&g.as_ref().unwrap().1)
    }

    pub fn all_pids() -> Vec<i32> {
        with_table(true, |t| t.keys().copied().filter(|p| *p > 0).collect())
    }

    struct Proc(HANDLE);
    impl Drop for Proc {
        fn drop(&mut self) {
            // SAFETY: we own the handle.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn open(pid: i32) -> Option<Proc> {
        // SAFETY: plain call; the handle is checked.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid as u32) };
        (!h.is_null()).then_some(Proc(h))
    }

    pub fn read_stat(pid: i32) -> Option<ProcStat> {
        let (ppid, comm) = with_table(false, |t| t.get(&pid).cloned())
            .or_else(|| with_table(true, |t| t.get(&pid).cloned()))?;
        let p = open(pid)?;
        let (mut c, mut e, mut k, mut u) = Default::default();
        // SAFETY: four FILETIME out parameters.
        if unsafe { GetProcessTimes(p.0, &mut c, &mut e, &mut k, &mut u) } == 0 {
            return None;
        }
        let rss = {
            use windows_sys::Win32::System::ProcessStatus::{
                K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
            };
            // SAFETY: the counters struct is plain data sized by cb.
            unsafe {
                let mut m: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
                m.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
                if K32GetProcessMemoryInfo(p.0, &mut m, m.cb) != 0 {
                    m.WorkingSetSize as u64
                } else {
                    0
                }
            }
        };
        let created_unix = ft(c).saturating_sub(EPOCH_DIFF) as f64 / TICKS;
        let since_boot = (created_unix - boot_time().unwrap_or(0.0)).max(0.0);
        Some(ProcStat {
            pid,
            ppid,
            comm,
            cpu_ticks: ft(k) + ft(u),
            rss_pages: rss / super::page_size(),
            start_ticks: (since_boot * TICKS) as u64,
        })
    }

    /// The command line from the process's own parameters.
    pub fn cmdline(pid: i32) -> String {
        use windows_sys::Wdk::System::Threading::{
            NtQueryInformationProcess, ProcessCommandLineInformation,
        };
        use windows_sys::Win32::Foundation::UNICODE_STRING;
        let Some(p) = open(pid) else {
            return String::new();
        };
        let mut buf = vec![0u64; 4096];
        let mut len = 0u32;
        // SAFETY: the buffer is writable for its stated byte length and
        // 8-aligned; on success it starts with a UNICODE_STRING pointing
        // into itself.
        unsafe {
            let st = NtQueryInformationProcess(
                p.0,
                ProcessCommandLineInformation,
                buf.as_mut_ptr().cast(),
                (buf.len() * 8) as u32,
                &mut len,
            );
            if st < 0 {
                return String::new();
            }
            let us = &*(buf.as_ptr() as *const UNICODE_STRING);
            if us.Buffer.is_null() || us.Length == 0 {
                return String::new();
            }
            let s = std::slice::from_raw_parts(us.Buffer, us.Length as usize / 2);
            String::from_utf16_lossy(s)
        }
    }
}

/// NUL-separated arguments as one space-separated line.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn join_args(b: &[u8]) -> String {
    b.split(|c| *c == 0)
        .filter(|p| !p.is_empty())
        .map(|p| String::from_utf8_lossy(p).into_owned())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(all(test, target_os = "linux"))]
#[path = "procfs_output_tests.rs"]
mod output_consumer_tests;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_holder_of_a_lock_a_build_is_blocked_on_is_found() {
        // Lock 2 has two waiters, lock 3 none; -1 is an open-file lock.
        let locks = "\
1: FLOCK  ADVISORY  WRITE 2176848 fe:10:1484522 0 EOF
2: FLOCK  ADVISORY  WRITE 2950552 fe:10:3181970 0 EOF
2: -> FLOCK  ADVISORY  WRITE 700 fe:10:3181970 0 EOF
2: -> FLOCK  ADVISORY  WRITE 701 fe:10:3181970 0 EOF
3: FLOCK  ADVISORY  READ 2950552 fe:10:3181968 0 EOF
4: OFDLCK ADVISORY  WRITE -1 fe:10:99 0 EOF
4: -> OFDLCK ADVISORY  WRITE -1 fe:10:99 0 EOF
5: POSIX  ADVISORY  WRITE 800 fe:10:5 0 EOF
5: -> POSIX  ADVISORY  WRITE 700 fe:10:5 0 EOF
";
        assert_eq!(parse_lock_holders(locks, |p| p == 700), [2950552, 800]);
        // Both waiters are ours: the holder is named once.
        assert_eq!(
            parse_lock_holders(locks, |p| p == 700 || p == 701),
            [2950552, 800]
        );
        // Our own process holding it is not somebody else's lock.
        assert_eq!(
            parse_lock_holders(locks, |p| p == 700 || p == 800),
            [2950552]
        );
        assert!(parse_lock_holders(locks, |p| p == 1).is_empty());
        assert!(parse_lock_holders("garbage\n\n9:\n", |_| true).is_empty());
    }

    /// The kernel's own format: a child really blocked on a lock we hold.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_really_blocked_child_shows_its_holder_in_proc_locks() {
        let path = std::env::temp_dir().join(format!("jr-held-{}.lock", std::process::id()));
        let held = std::fs::File::create(&path).unwrap();
        held.lock().unwrap();
        let Ok(mut child) = std::process::Command::new("flock")
            .arg(&path)
            .arg("true")
            .spawn()
        else {
            return; // no flock(1) here
        };
        let (me, kid) = (std::process::id(), child.id());
        let mut found = Vec::new();
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(20));
            let locks = std::fs::read_to_string("/proc/locks").unwrap();
            found = parse_lock_holders(&locks, |p| p == kid);
            if !found.is_empty() {
                break;
            }
        }
        drop(held);
        let _ = child.wait();
        let _ = std::fs::remove_file(&path);
        assert_eq!(found, [me]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn child_hashes_running_image_after_unlink() {
        const KEY: &str = "JUSTRUST_TEST_UNLINKED_IMAGE";
        if let Ok(copy) = std::env::var(KEY) {
            // Never unlink an arbitrary path supplied in the environment.
            let exe = std::env::current_exe().unwrap();
            assert_eq!(exe, std::path::Path::new(&copy).join("test-bin"));
            let expected = crate::toolchain::sha256(&exe).unwrap();
            std::fs::remove_file(&exe).unwrap();
            std::fs::write(&exe, b"replacement, not the running image").unwrap();
            assert_eq!(
                crate::toolchain::sha256(&running_executable_path().unwrap()).unwrap(),
                expected
            );
            return;
        }
        let dir = std::env::temp_dir().join(format!("jr-unlinked-{}", std::process::id()));
        std::fs::create_dir(&dir).unwrap();
        let copy = dir.join("test-bin");
        let exe = std::env::current_exe().unwrap();
        if std::fs::hard_link(&exe, &copy).is_err() {
            std::fs::copy(exe, &copy).unwrap();
        }
        // A copy was just written: a child another test is forking can hold
        // it open for writing until it execs.
        // Captured: the nested harness would otherwise write its own result
        // lines into this process's stdout, between this harness's lines.
        let result = crate::test_fork::exec_fresh(|| {
            std::process::Command::new(&copy)
                .args([
                    "--exact",
                    "procfs::tests::child_hashes_running_image_after_unlink",
                ])
                .env(KEY, &dir)
                .output()
        });
        let replaced = std::fs::metadata(&copy).unwrap().len() < 100;
        std::fs::remove_dir_all(dir).unwrap();
        let result = result.unwrap();
        assert!(
            result.status.success() && replaced,
            "replaced {replaced}\n{}{}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn only_the_user_manager_argv_is_session_plumbing() {
        let yes: [&[&[u8]]; 4] = [
            &[b"/usr/lib/systemd/systemd", b"--user"],
            &[b"/lib/systemd/systemd", b"--user"],
            &[b"(sd-pam)"],
            &[b"sshd: ubuntu@notty                "],
        ];
        for argv in yes {
            assert!(is_user_manager_argv(argv), "{argv:?}");
        }
        let no: [&[&[u8]]; 7] = [
            &[b"sshd: listener"],
            &[b"cargo", b"build"],
            &[b"/usr/lib/systemd/systemd", b"--user", b"--extra"],
            &[b"/tmp/evil/systemd-like", b"--user"],
            &[b"/usr/lib/systemd/systemd"],
            &[b"sd-pam"],
            &[],
        ];
        for argv in no {
            assert!(!is_user_manager_argv(argv), "{argv:?}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn ancestors_are_the_parent_chain() {
        let ancestors = ancestors_of_self();
        let parent = std::os::unix::process::parent_id();
        if parent > 1 {
            assert_eq!(ancestors.first(), Some(&parent));
        }
        assert!(!ancestors.contains(&std::process::id()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn readable_processes_are_never_exempt_and_live_use_is_seen() {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("jr-inuse-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dir = dir.canonicalize().unwrap();
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .current_dir(&dir)
            .spawn()
            .unwrap();
        let child_base = std::path::PathBuf::from(format!("/proc/{}", child.id()));
        assert!(!is_session_plumbing(&child_base, &child.id().to_string()));
        assert!(
            path_in_use_by_other(&dir),
            "a child with its cwd inside is live use"
        );
        child.kill().unwrap();
        child.wait().unwrap();
        // No "unused afterwards" assertion: a parallel test process that is
        // momentarily unreadable (mid-exec) is correctly treated as possible
        // use, so that check is nondeterministic by design.
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn umask_parser_accepts_only_permission_bits() {
        for (value, expected) in [
            ("0000", 0),
            ("0002", 0o002),
            ("0022", 0o022),
            ("0077", 0o077),
            ("777", 0o777),
        ] {
            assert_eq!(parse_umask(value), Some(expected));
        }
        for value in [
            "",
            "-022",
            "+022",
            "888",
            "1777",
            "00022",
            "0o22",
            "022;touch x",
            "0022\n",
        ] {
            assert_eq!(parse_umask(value), None, "{value}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn caller_umask_is_read_without_changing_it() {
        let read_shell = || {
            let out = std::process::Command::new("sh")
                .args(["-c", "umask"])
                .output()
                .unwrap();
            assert!(out.status.success());
            parse_umask(std::str::from_utf8(&out.stdout).unwrap().trim()).unwrap()
        };
        let before = read_shell();
        assert_eq!(current_umask(), Some(before));
        assert_eq!(read_shell(), before);
    }

    /// A child spawned through `close_inherited_fds` does not keep a flock
    /// its parent held (the bench-lock leak through `justrust __daemon`).
    #[cfg(unix)]
    #[test]
    fn daemon_child_does_not_keep_inherited_flock() {
        use std::os::fd::AsRawFd;
        let p = std::env::temp_dir().join(format!("jr-fdleak-{}", std::process::id()));
        let lock = |f: &std::fs::File| unsafe {
            libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) == 0
        };
        let spawn_sleeper = |p: &std::path::Path, close: bool| {
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
                use std::os::unix::process::CommandExt;
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

    /// Regression test: `close_inherited_fds` must not close Rust std's own
    /// internal exec-error pipe fd, otherwise a failed `execve` (e.g. ENOENT
    /// after binary replacement) aborts the child with SIGABRT instead of
    /// returning an `io::Error`.
    #[cfg(unix)]
    #[test]
    fn close_inherited_fds_reports_exec_failure_cleanly() {
        let mut c = std::process::Command::new("/nonexistent/justrust-close-range-test");
        close_inherited_fds(&mut c);
        let err = c
            .spawn()
            .expect_err("nonexistent binary must fail to spawn");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound);
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

    #[cfg(target_os = "linux")]
    #[test]
    fn installed_memory_counts_ballooned_blocks() {
        // The hosted guest: 236 online 128-MiB blocks while the balloon
        // left MemTotal at 5.4 GiB.
        let states = (0..240).map(|i| if i < 236 { "online\n" } else { "offline\n" }.to_string());
        assert_eq!(super::linux::online_mb("8000000\n", states), Some(30208));
        assert_eq!(super::linux::online_mb("8000000", std::iter::empty()), None);
        assert_eq!(
            super::linux::online_mb("zz", ["online".to_string()].into_iter()),
            None
        );
        let m = meminfo().unwrap();
        assert!(installed_mb().unwrap() >= m.total_mb);
    }

    /// A child's tree link, CPU, start, and argv are read like `record`
    /// and `status` use them.
    #[cfg(unix)]
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

    /// Windows: the child's parent, name, argv, and start are read.
    #[cfg(windows)]
    #[test]
    fn reads_child_process() {
        let mut c = std::process::Command::new("ping")
            .args(["-n", "5", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = c.id() as i32;
        let st = read_stat(pid).expect("child stat");
        assert_eq!(st.ppid, std::process::id() as i32);
        assert_eq!(st.comm.to_lowercase(), "ping");
        assert!(all_pids().contains(&pid));
        // Wine has no ProcessCommandLineInformation: empty there, never wrong.
        let argv = cmdline(pid);
        let wine =
            std::env::var_os("WINEPREFIX").is_some() || std::env::var_os("WINELOADER").is_some();
        assert!(
            argv.contains("127.0.0.1") || (wine && argv.is_empty()),
            "{argv}"
        );
        let started = boot_time().unwrap() + st.start_ticks as f64 / clk_tck();
        assert!((crate::paths::now() - started).abs() < 60.0);
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
