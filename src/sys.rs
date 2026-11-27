//! Small platform layer: the few process, file, and socket operations whose
//! Unix and Windows forms differ. Unix keeps the exact calls justrust always
//! made (exec, process groups, permission bits, inode stamps); Windows gets
//! the closest equivalent, or a documented no-op where none exists.
//!
//! Larger platform splits live in their own modules (`procfs` for process
//! and machine sampling, `remote_watch` for file watching).

use std::fs::Metadata;
use std::io;
use std::path::Path;
use std::process::{Command, ExitStatus};

/// Whether a system battery explicitly reports that it is discharging.
/// Optional eager cache transfers can wait for AC power. Unknown state and
/// unsupported platforms preserve existing behavior; peripheral batteries do
/// not determine the host's power policy.
pub fn battery_discharging() -> bool {
    #[cfg(target_os = "linux")]
    {
        battery_discharging_at(Path::new("/sys/class/power_supply"))
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(any(target_os = "linux", test))]
fn battery_discharging_at(root: &Path) -> bool {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .any(|entry| {
            let path = entry.path();
            let read = |name| std::fs::read_to_string(path.join(name)).unwrap_or_default();
            read("type").trim() == "Battery"
                && read("scope").trim() != "Device"
                && read("status").trim() == "Discharging"
        })
}

/// Local stream sockets: Unix domain sockets on Unix, AF_UNIX sockets on
/// Windows 10 1803+ (`uds_windows`), with the same API surface.
pub mod net {
    #[cfg(unix)]
    pub use std::os::unix::net::{UnixListener, UnixStream};
    #[cfg(windows)]
    pub use uds_windows::{UnixListener, UnixStream};
}

/// Replace this process with `cmd`. Windows has no exec: run the command
/// with inherited stdio, ignoring Ctrl+C here (the child, in the same
/// console, gets it and decides), and exit with its code. Returns only on
/// failure to start.
pub fn exec(cmd: &mut Command) -> io::Error {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.exec()
    }
    #[cfg(windows)]
    {
        ignore_ctrl_c();
        match cmd.status() {
            Ok(s) => std::process::exit(exit_code(s)),
            Err(e) => e,
        }
    }
}

/// Start a command detached from the caller's terminal and its Ctrl+C:
/// its own process group on Unix, a new process group without a console
/// window on Windows. A trait so builder chains keep reading as before.
pub trait Detach {
    fn detached(&mut self) -> &mut Self;
}

impl Detach for Command {
    fn detached(&mut self) -> &mut Command {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            self.process_group(0)
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use windows_sys::Win32::System::Threading::{
                CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
            };
            self.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
        }
    }
}

/// Ask the process group led by `pid` (started with `detached()`) to stop:
/// SIGTERM, or SIGINT when `interrupt`. Windows has no signals for
/// windowless children: Ctrl+Break reaches a new process group only when
/// it shares our console, so this terminates the process instead.
pub fn terminate_group(pid: i32, interrupt: bool) {
    #[cfg(unix)]
    // SAFETY: kill is async-signal-safe and only signals.
    unsafe {
        libc::kill(
            -pid,
            if interrupt {
                libc::SIGINT
            } else {
                libc::SIGTERM
            },
        );
    }
    #[cfg(windows)]
    {
        let _ = interrupt;
        terminate(pid);
    }
}

/// Stop process `pid` now (TerminateProcess).
#[cfg(windows)]
pub fn terminate(pid: i32) {
    // SAFETY: the handle is checked and closed.
    unsafe {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::{
            OpenProcess, PROCESS_TERMINATE, TerminateProcess,
        };
        let h = OpenProcess(PROCESS_TERMINATE, 0, pid as u32);
        if !h.is_null() {
            TerminateProcess(h, 1);
            CloseHandle(h);
        }
    }
}

/// Exit code the way a shell reports it: signals as 128+n on Unix.
pub fn exit_code(s: ExitStatus) -> i32 {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0))
    }
    #[cfg(windows)]
    {
        s.code().unwrap_or(1)
    }
}

/// Lowest CPU (and on Windows I/O) priority for background maintenance.
pub fn lower_priority() {
    // SAFETY: plain calls on the current process.
    #[cfg(unix)]
    unsafe {
        libc::setpriority(libc::PRIO_PROCESS, 0, 19);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{
            GetCurrentProcess, PROCESS_MODE_BACKGROUND_BEGIN, SetPriorityClass,
        };
        SetPriorityClass(GetCurrentProcess(), PROCESS_MODE_BACKGROUND_BEGIN);
    }
}

/// Is stdin (0), stdout (1), or stderr (2) a terminal?
pub fn isatty(fd: i32) -> bool {
    use std::io::IsTerminal;
    match fd {
        0 => io::stdin().is_terminal(),
        1 => io::stdout().is_terminal(),
        2 => io::stderr().is_terminal(),
        _ => false,
    }
}

/// Columns of the terminal on `fd` (1 or 2).
pub fn term_width(fd: i32) -> Option<u16> {
    #[cfg(unix)]
    // SAFETY: winsize is plain data and only written by the ioctl.
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        (libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) == 0 && ws.ws_col > 0).then_some(ws.ws_col)
    }
    #[cfg(windows)]
    // SAFETY: the info struct is plain data written by the call.
    unsafe {
        use windows_sys::Win32::System::Console::{
            CONSOLE_SCREEN_BUFFER_INFO, GetConsoleScreenBufferInfo, GetStdHandle, STD_ERROR_HANDLE,
            STD_OUTPUT_HANDLE,
        };
        let h = GetStdHandle(if fd == 1 {
            STD_OUTPUT_HANDLE
        } else {
            STD_ERROR_HANDLE
        });
        let mut info: CONSOLE_SCREEN_BUFFER_INFO = std::mem::zeroed();
        if GetConsoleScreenBufferInfo(h, &mut info) == 0 {
            return None;
        }
        let w = info.srWindow.Right - info.srWindow.Left + 1;
        (w > 0).then_some(w as u16)
    }
}

/// Keep running through Ctrl+C / Ctrl+Break so a summary still gets
/// written: the console delivers them to the child (cargo) as well.
#[cfg(windows)]
pub fn ignore_ctrl_c() {
    // SAFETY: a null handler with TRUE means "ignore Ctrl+C" for this process.
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(None, 1);
    }
}

/// The terminal session this process belongs to: the Unix session id, or
/// on Windows the console's first process (the shell that owns it), so
/// commands from one terminal share it.
pub fn session_id() -> u64 {
    #[cfg(unix)]
    // SAFETY: getsid has no memory-safety preconditions.
    unsafe {
        libc::getsid(0) as u64
    }
    #[cfg(windows)]
    // SAFETY: the buffer is writable for its stated length.
    unsafe {
        let mut pids = [0u32; 64];
        let n = windows_sys::Win32::System::Console::GetConsoleProcessList(
            pids.as_mut_ptr(),
            pids.len() as u32,
        ) as usize;
        // Most recently attached first: the owner is the last one listed.
        pids[..n.min(pids.len())]
            .last()
            .map_or(std::process::id(), |p| *p) as u64
    }
}

/// A per-user name for shared temporary paths: the uid on Unix, the user
/// name on Windows.
pub fn user_tag() -> String {
    #[cfg(unix)]
    // SAFETY: getuid has no preconditions.
    unsafe {
        libc::getuid().to_string()
    }
    #[cfg(windows)]
    {
        std::env::var("USERNAME").unwrap_or_else(|_| "user".into())
    }
}

// ------------------------------------------------------------------ files

/// Does `path` (as text) contain the directory component `dir`, with
/// either separator? `has_dir(p, "deps")` matches `/x/deps/y` and
/// `C:\x\deps\y`.
pub fn has_dir(path: &str, dir: &str) -> bool {
    path.split(['/', '\\']).any(|c| c == dir) && last_component(path) != dir
}

/// The last component of `path` (as text), with either separator.
pub fn last_component(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The null device, for tools that take a path (`curl -o`, ssh options).
pub const DEV_NULL: &str = if cfg!(windows) { "NUL" } else { "/dev/null" };

/// Modification time as (seconds, nanoseconds) since the Unix epoch.
pub fn mtime(m: &Metadata) -> (i64, i64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (m.mtime(), m.mtime_nsec())
    }
    #[cfg(not(unix))]
    {
        match m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        {
            Some(d) => (d.as_secs() as i64, d.subsec_nanos() as i64),
            None => (0, 0),
        }
    }
}

/// Modification time in seconds since the epoch, with fractions.
pub fn mtime_f64(m: &Metadata) -> f64 {
    let (s, ns) = mtime(m);
    s as f64 + ns as f64 / 1e9
}

/// Identity of the file a path names now: (device, inode) on Unix. Windows
/// has no stable equivalent in std, so size, last write and creation time
/// stand in. Creation time alone does not tell a replaced file from the
/// original: NTFS gives a file renamed over a name the creation time of the
/// file that held the name just before (tunneling).
pub fn file_id(m: &Metadata) -> (u64, u64) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        (m.dev(), m.ino())
    }
    #[cfg(not(unix))]
    {
        let created = m
            .created()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64);
        fallback_file_id(m.len(), mtime(m), created)
    }
}

/// `file_id` without an inode. The second field stays the creation time:
/// file stamps hash it next to the size and write time they already carry.
#[cfg(any(not(unix), test))]
fn fallback_file_id(len: u64, written: (i64, i64), created: u64) -> (u64, u64) {
    let written = (written.0 as u64)
        .wrapping_mul(1_000_000_000)
        .wrapping_add(written.1 as u64);
    (
        len ^ written.wrapping_mul(0x9E37_79B9_7F4A_7C15).rotate_left(17),
        created,
    )
}

/// Bytes the file occupies on disk (allocated blocks on Unix, the length
/// elsewhere).
pub fn disk_bytes(m: &Metadata) -> u64 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        m.blocks() * 512
    }
    #[cfg(not(unix))]
    {
        m.len()
    }
}

/// Unix permission bits. Windows: 0o755 for executables (by extension),
/// 0o644 otherwise, without write bits for read-only files.
pub fn mode(m: &Metadata, path: &Path) -> u32 {
    #[cfg(unix)]
    {
        let _ = path;
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode()
    }
    #[cfg(not(unix))]
    {
        let base = if has_exe_extension(path) {
            0o755
        } else {
            0o644
        };
        if m.permissions().readonly() {
            base & !0o222
        } else {
            base
        }
    }
}

/// Set Unix permission bits. Windows keeps only the read-only flag.
pub fn set_mode(path: &Path, mode: u32) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
    }
    #[cfg(not(unix))]
    {
        let mut p = std::fs::metadata(path)?.permissions();
        p.set_readonly(mode & 0o200 == 0);
        std::fs::set_permissions(path, p)
    }
}

#[cfg(not(unix))]
fn has_exe_extension(p: &Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["exe", "com", "bat", "cmd"]
            .iter()
            .any(|x| e.eq_ignore_ascii_case(x))
    })
}

/// `(free, total)` bytes on the filesystem holding `dir` (`free` is bytes
/// available to this user).
pub fn disk_space(dir: &Path) -> Option<(u64, u64)> {
    #[cfg(unix)]
    {
        let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).ok()?;
        // SAFETY: statvfs fills the zeroed struct; the path is NUL-terminated.
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        let frsize = st.f_frsize as u64;
        Some((
            (st.f_bavail as u64).saturating_mul(frsize),
            (st.f_blocks as u64).saturating_mul(frsize),
        ))
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let w: Vec<u16> = dir.as_os_str().encode_wide().chain([0]).collect();
        let mut free = 0u64;
        let mut total = 0u64;
        // SAFETY: NUL-terminated wide path; optional outputs may be null.
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW(
                w.as_ptr(),
                &mut free,
                &mut total,
                std::ptr::null_mut(),
            )
        };
        (ok != 0).then_some((free, total))
    }
}

/// A file a program can be run from.
pub fn is_executable(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else {
        return false;
    };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        has_exe_extension(p)
    }
}

/// `name` as an executable file name on this platform (`name.exe` on Windows).
pub fn exe_name(name: &str) -> String {
    format!("{name}{}", std::env::consts::EXE_SUFFIX)
}

/// A program name from argv[0] or a path: the file name, without `.exe` on
/// Windows.
pub fn program_name(p: &std::ffi::OsStr) -> String {
    let n = Path::new(p)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if cfg!(windows) && n.len() > 4 && n[n.len() - 4..].eq_ignore_ascii_case(".exe") {
        return n[..n.len() - 4].to_owned();
    }
    n
}

/// Open for writing a new file readable only by its owner (0600 on Unix;
/// Windows files inherit the profile directory's ACL, private already).
pub fn create_private(path: &Path) -> io::Result<std::fs::File> {
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        o.mode(0o600);
    }
    o.open(path)
}

/// Fill `buf` from `f` at `offset` without moving a shared cursor.
pub fn read_exact_at(f: &std::fs::File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        f.read_exact_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            match f.seek_read(&mut buf[done..], offset + done as u64) {
                Ok(0) => return Err(io::ErrorKind::UnexpectedEof.into()),
                Ok(n) => done += n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}

/// Make `link` name `target`: a symlink on Unix. Windows symlinks need
/// developer mode or admin rights, so a directory gets a junction-free
/// fallback of `Err`, and a file a hard link (same volume), else a copy.
pub fn symlink(target: &Path, link: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        if target.is_dir() {
            return std::os::windows::fs::symlink_dir(target, link);
        }
        std::os::windows::fs::symlink_file(target, link)
            .or_else(|_| std::fs::hard_link(target, link))
            .or_else(|_| std::fs::copy(target, link).map(|_| ()))
    }
}

// ------------------------------------------------------------------ processes

/// CPU and memory of a finished process tree.
#[derive(Debug, Clone, Copy, Default)]
pub struct Usage {
    pub code: i32,
    pub user: f64,
    pub sys: f64,
    pub max_rss_mb: f64,
}

/// The output pipe of a child process, read to the end of what the child
/// itself wrote.
pub trait OutputPipe: io::Read + Send + 'static {
    /// Whether to keep reading: `false` once `exited` was set (the child was
    /// reaped) and the pipe is empty and still open. Everything the child
    /// wrote is in the pipe before it is reaped, so what remains open then is
    /// a process the child started that inherited the pipe, and may hold it
    /// for as long as it lives. Blocks until there is something to read, the
    /// pipe closes, or that is known.
    fn more(&self, exited: &std::sync::atomic::AtomicBool) -> bool;
}

#[cfg(unix)]
fn pipe_more(fd: std::os::fd::RawFd, exited: &std::sync::atomic::AtomicBool) -> bool {
    // Once the child is reaped and its pipe is empty, how long the pipe may
    // stay open before that means a process the child left behind.
    let mut grace = PIPE_HELD_GRACE_MS;
    loop {
        // Read the flag first: an empty pipe only means "all read" when the
        // child had already exited before the pipe was looked at.
        let done = exited.load(std::sync::atomic::Ordering::Acquire);
        let mut p = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        let wait = if done {
            grace.min(PIPE_HELD_STEP_MS)
        } else {
            50
        };
        // SAFETY: one valid pollfd.
        let r = unsafe { libc::poll(&mut p, 1, wait) };
        if r > 0 {
            // Data, or every writer closed: `read` tells which.
            return true;
        }
        if r < 0 && io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
            return true;
        }
        if r == 0 && done {
            // Open and empty is also what a process being started by another
            // thread looks like: between its fork and its exec it holds a
            // copy of every descriptor, close-on-exec ones included. Under
            // load that window is long enough to be seen here, so only a
            // pipe that stays open is one somebody keeps.
            if grace <= 0 {
                return false;
            }
            grace -= wait.max(1);
        }
    }
}

/// See [`pipe_more`]: far longer than a fork-to-exec window, far shorter than
/// anything worth not waiting for.
#[cfg(unix)]
const PIPE_HELD_GRACE_MS: i32 = 250;
#[cfg(unix)]
const PIPE_HELD_STEP_MS: i32 = 10;

#[cfg(unix)]
impl OutputPipe for std::process::ChildStdout {
    fn more(&self, exited: &std::sync::atomic::AtomicBool) -> bool {
        pipe_more(std::os::fd::AsRawFd::as_raw_fd(self), exited)
    }
}

#[cfg(unix)]
impl OutputPipe for std::process::ChildStderr {
    fn more(&self, exited: &std::sync::atomic::AtomicBool) -> bool {
        pipe_more(std::os::fd::AsRawFd::as_raw_fd(self), exited)
    }
}

// On Windows the child's tree is in a job object and the pipe is read to its
// end, as before.
#[cfg(windows)]
impl OutputPipe for std::process::ChildStdout {
    fn more(&self, _exited: &std::sync::atomic::AtomicBool) -> bool {
        true
    }
}

#[cfg(windows)]
impl OutputPipe for std::process::ChildStderr {
    fn more(&self, _exited: &std::sync::atomic::AtomicBool) -> bool {
        true
    }
}

/// No job objects on Unix: `wait4` already counts the waited-for
/// descendants (linker, `cc`) of the child.
#[cfg(unix)]
pub struct Job;

#[cfg(unix)]
impl Job {
    pub fn for_child(_child: &std::process::Child) -> Option<Job> {
        None
    }
}

/// Wait for `child` with `wait4`: exit code (signals as 128+n), user and
/// system CPU of it and its waited-for descendants, and max RSS.
#[cfg(unix)]
pub fn wait_usage(child: &mut std::process::Child, _job: Option<&Job>) -> Usage {
    let pid = child.id() as i32;
    let mut status = 0;
    // SAFETY: rusage is plain data and fully written by wait4 on success.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    loop {
        // SAFETY: valid pointers to local storage.
        let r = unsafe { libc::wait4(pid, &mut status, 0, &mut ru) };
        if r == pid {
            break;
        }
        if r == -1 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Usage {
            code: 1,
            ..Usage::default()
        };
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
    let max_rss_mb = if cfg!(target_os = "macos") {
        ru.ru_maxrss as f64 / 1_048_576.0
    } else {
        ru.ru_maxrss as f64 / 1024.0
    };
    Usage {
        code,
        user: tv(ru.ru_utime),
        sys: tv(ru.ru_stime),
        max_rss_mb,
    }
}

/// User plus system CPU seconds of this process so far.
pub fn self_cpu() -> f64 {
    #[cfg(unix)]
    // SAFETY: getrusage fills a plain struct.
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut ru);
        let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        tv(ru.ru_utime) + tv(ru.ru_stime)
    }
    #[cfg(windows)]
    // SAFETY: four FILETIME out parameters for our own process.
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        let ft = |t: windows_sys::Win32::Foundation::FILETIME| {
            ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e7
        };
        let (mut c, mut e, mut k, mut u) = Default::default();
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) == 0 {
            return 0.0;
        }
        ft(k) + ft(u)
    }
}

/// CPU of every descendant this process has waited for (`RUSAGE_CHILDREN`).
#[cfg(unix)]
pub fn children_cpu() -> (f64, f64) {
    // SAFETY: rusage is plain data written by getrusage.
    let mut kids: libc::rusage = unsafe { std::mem::zeroed() };
    unsafe {
        libc::getrusage(libc::RUSAGE_CHILDREN, &mut kids);
    }
    let tv = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
    (tv(kids.ru_utime), tv(kids.ru_stime))
}

/// A Windows job object holding a child and everything it starts, for CPU
/// totals over the whole tree (what `wait4` and `RUSAGE_CHILDREN` give on
/// Unix). Children started before `assign` runs are not counted.
#[cfg(windows)]
pub struct Job(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
// SAFETY: a job handle may be used from any thread.
unsafe impl Send for Job {}
#[cfg(windows)]
// SAFETY: as above; the queries do not mutate shared state.
unsafe impl Sync for Job {}

#[cfg(windows)]
impl Job {
    pub fn for_child(child: &std::process::Child) -> Option<Job> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::{AssignProcessToJobObject, CreateJobObjectW};
        // SAFETY: an anonymous job with default security; handles checked.
        unsafe {
            let h = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if h.is_null() {
                return None;
            }
            let job = Job(h);
            (AssignProcessToJobObject(h, child.as_raw_handle()) != 0).then_some(job)
        }
    }

    /// Total (user, kernel) CPU seconds of every process ever in the job.
    pub fn cpu(&self) -> (f64, f64) {
        use windows_sys::Win32::System::JobObjects::{
            JOBOBJECT_BASIC_ACCOUNTING_INFORMATION, JobObjectBasicAccountingInformation,
            QueryInformationJobObject,
        };
        // SAFETY: the out buffer is the struct the class asks for.
        unsafe {
            let mut info: JOBOBJECT_BASIC_ACCOUNTING_INFORMATION = std::mem::zeroed();
            let ok = QueryInformationJobObject(
                self.0,
                JobObjectBasicAccountingInformation,
                (&mut info as *mut JOBOBJECT_BASIC_ACCOUNTING_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_BASIC_ACCOUNTING_INFORMATION>() as u32,
                std::ptr::null_mut(),
            );
            if ok == 0 {
                return (0.0, 0.0);
            }
            (
                info.TotalUserTime as f64 / 1e7,
                info.TotalKernelTime as f64 / 1e7,
            )
        }
    }
}

#[cfg(windows)]
impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: we own the handle.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.0) };
    }
}

/// Peak working set of a (finished, not yet dropped) child, in MiB.
#[cfg(windows)]
pub fn peak_rss_mb(child: &std::process::Child) -> f64 {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::ProcessStatus::{
        K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
    };
    // SAFETY: the counters struct is plain data sized by cb.
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
        if K32GetProcessMemoryInfo(child.as_raw_handle(), &mut c, c.cb) == 0 {
            return 0.0;
        }
        c.PeakWorkingSetSize as f64 / 1_048_576.0
    }
}

/// Wait for `child` and report its tree's CPU (from `job` when there is
/// one, else the child's own times) and its peak working set.
#[cfg(windows)]
pub fn wait_usage(child: &mut std::process::Child, job: Option<&Job>) -> Usage {
    let code = match child.wait() {
        Ok(s) => exit_code(s),
        Err(_) => {
            return Usage {
                code: 1,
                ..Usage::default()
            };
        }
    };
    // The process handle stays valid until `child` drops, so its own
    // times are still readable here. Job totals cover the whole tree; use
    // them unless the job reports less (no job, or no accounting).
    let own = process_cpu(child);
    let (user, sys) = match job.map(Job::cpu) {
        Some((u, s)) if u + s >= own.0 + own.1 => (u, s),
        _ => own,
    };
    Usage {
        code,
        user,
        sys,
        max_rss_mb: peak_rss_mb(child),
    }
}

#[cfg(windows)]
fn process_cpu(child: &std::process::Child) -> (f64, f64) {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::GetProcessTimes;
    let ft = |t: FILETIME| ((t.dwHighDateTime as u64) << 32 | t.dwLowDateTime as u64) as f64 / 1e7;
    // SAFETY: four FILETIME out parameters.
    unsafe {
        let (mut c, mut e, mut k, mut u) = std::mem::zeroed();
        if GetProcessTimes(child.as_raw_handle(), &mut c, &mut e, &mut k, &mut u) == 0 {
            return (0.0, 0.0);
        }
        (ft(u), ft(k))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_file_id_tells_a_same_size_replacement_with_tunneled_creation_time() {
        let created = 1_795_000_000_000_000_000;
        let original = fallback_file_id(2, (1_795_000_000, 100), created);
        assert_eq!(original, fallback_file_id(2, (1_795_000_000, 100), created));
        // Same name, size and (tunneled) creation time, written later.
        let replaced = fallback_file_id(2, (1_795_000_000, 200), created);
        assert_ne!(original, replaced);
        assert_ne!(original, fallback_file_id(3, (1_795_000_000, 100), created));
        // File stamps keep hashing the creation time.
        assert_eq!(original.1, created);
        assert_eq!(replaced.1, created);
    }

    #[test]
    fn only_discharging_system_batteries_defer_eager_transfers() {
        let root = std::env::temp_dir().join(format!("jr-sys-power-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        assert!(!battery_discharging_at(&root), "unknown platform state");
        let battery = root.join("BAT0");
        std::fs::create_dir_all(&battery).unwrap();
        assert!(!battery_discharging_at(&root), "missing attributes");
        for (kind, scope, status, expected) in [
            ("Battery\n", "System\n", "Discharging\n", true),
            ("Battery", "System", "Charging", false),
            ("Battery", "System", "Full", false),
            ("Battery", "System", "Not charging", false),
            ("Battery", "System", "Unknown", false),
            ("Battery", "Device", "Discharging", false),
            ("Mains", "System", "Discharging", false),
            ("USB", "System", "Discharging", false),
            ("Battery", "", "Discharging", true),
        ] {
            std::fs::write(battery.join("type"), kind).unwrap();
            std::fs::write(battery.join("scope"), scope).unwrap();
            std::fs::write(battery.join("status"), status).unwrap();
            assert_eq!(
                battery_discharging_at(&root),
                expected,
                "{kind:?} {scope:?} {status:?}"
            );
        }
        std::fs::remove_file(battery.join("scope")).unwrap();
        assert!(battery_discharging_at(&root), "older kernels omit scope");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn path_components_with_either_separator() {
        assert!(has_dir("/w/target/debug/deps/app-1", "deps"));
        assert!(has_dir(r"D:\w\target\debug\deps\app-1.exe", "deps"));
        assert!(!has_dir("/w/target/debug/deps", "deps"));
        assert!(!has_dir("/w/deps-x/app", "deps"));
        assert_eq!(last_component(r"C:\t\deps\x-1.exe"), "x-1.exe");
        assert_eq!(last_component("t/deps/x-1"), "x-1");
    }

    #[test]
    fn program_name_strips_exe_only_on_windows() {
        assert_eq!(program_name("/a/b/rustc".as_ref()), "rustc");
        assert_eq!(program_name("ld.lld".as_ref()), "ld.lld");
        if cfg!(windows) {
            assert_eq!(program_name(r"C:\x\rustc.EXE".as_ref()), "rustc");
        }
    }

    #[test]
    fn mtime_matches_modified() {
        let me = std::env::current_exe().unwrap();
        let m = std::fs::metadata(&me).unwrap();
        let want = m
            .modified()
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs_f64();
        assert!((mtime_f64(&m) - want).abs() < 1e-3);
        assert!(is_executable(&me));
    }

    #[test]
    fn read_at_offset() {
        let p = std::env::temp_dir().join(format!("jr-sys-read-{}", std::process::id()));
        std::fs::write(&p, b"0123456789").unwrap();
        let f = std::fs::File::open(&p).unwrap();
        let mut b = [0u8; 3];
        read_exact_at(&f, &mut b, 4).unwrap();
        assert_eq!(&b, b"456");
        assert!(read_exact_at(&f, &mut b, 9).is_err());
        drop(f);
        let _ = std::fs::remove_file(&p);
    }
}
