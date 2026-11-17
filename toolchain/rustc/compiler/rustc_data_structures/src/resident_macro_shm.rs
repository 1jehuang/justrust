//! Bounded Linux/x86-64 byte transport for an existing framed `Read + Write` session.
//!
//! This is deliberately not a framing, retry, or replay layer. A transport error must
//! fail the session. Both peers must be trusted: seals protect the mapping's extent,
//! not its contents against a malicious peer of the same user. In particular, peers
//! must not modify the immutable header or forge cursors after attachment.
//!
//! `create` returns the parent endpoint and a CLOEXEC descriptor for the child.
//! Pass a duplicate of that descriptor through the spawn implementation's explicit
//! inherited-fd mechanism, then authenticate the socket peer and call
//! `attach_child(fd, expected_creator_pid)`. Never clear CLOEXEC
//! globally around a concurrent spawn. Claims are permanent, including after drop.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::cell::{Cell, UnsafeCell};
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::mem::size_of;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

pub const CAPACITY: usize = 64 * 1024;
const MAGIC: [u8; 8] = *b"JRMSHM01";
const VERSION: u32 = 2;
const REQUIRED_SEALS: libc::c_int = libc::F_SEAL_SHRINK | libc::F_SEAL_GROW | libc::F_SEAL_SEAL;
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_TIMEOUT: Duration = Duration::from_secs(300);
const MAX_SPINS: u32 = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Role {
    Parent,
    Child,
}

impl Role {
    fn index(self) -> usize {
        match self {
            Self::Parent => 0,
            Self::Child => 1,
        }
    }
}

/// A timeout bounds each individual read/write, including spurious wakes and EINTR.
/// A timeout fails the endpoint permanently. No request can be silently replayed.
#[derive(Clone, Copy, Debug)]
pub struct Options {
    pub wait_timeout: Duration,
    pub spin_iterations: u32,
}

impl Default for Options {
    fn default() -> Self {
        Self { wait_timeout: DEFAULT_TIMEOUT, spin_iterations: 0 }
    }
}

#[repr(C, align(64))]
struct Ring {
    // Monotonically wrapping byte counts. The distance is always <= CAPACITY.
    head: AtomicU32,
    tail: AtomicU32,
    // Changes on either cursor movement or close. Used for lost-wake-free futex wait.
    event: AtomicU32,
    sleepers: AtomicU32,
    bytes: UnsafeCell<[u8; CAPACITY]>,
}

impl Ring {
    fn new() -> Self {
        Self {
            head: AtomicU32::new(0),
            tail: AtomicU32::new(0),
            event: AtomicU32::new(0),
            sleepers: AtomicU32::new(0),
            bytes: UnsafeCell::new([0; CAPACITY]),
        }
    }

    fn notify(&self) {
        // The SC handshake with wait() avoids a syscall on the hot path without
        // losing a wake: either the waiter sees the event change after registering,
        // or this load sees its registration and wakes it. Cursor publication itself
        // remains Release/Acquire. At most the two endpoints can register waiters.
        self.event.fetch_add(1, Ordering::SeqCst);
        if self.sleepers.load(Ordering::SeqCst) == 0 {
            return;
        }
        // Shared (not PRIVATE) futexes are essential for the process boundary.
        // Wake errors cannot strand a peer forever: every wait has a deadline.
        unsafe {
            libc::syscall(libc::SYS_futex, self.event.as_ptr(), libc::FUTEX_WAKE, i32::MAX);
        }
    }
}

#[repr(C, align(64))]
struct Shared {
    magic: [u8; 8],
    version: u32,
    mapping_size: u32,
    capacity: u32,
    owner_uid: u32,
    creator_pid: libc::pid_t,
    reserved: [u32; 1],
    claims: [AtomicU32; 2],
    closed: [AtomicU32; 2],
    rings: [Ring; 2],
}

struct Mapping {
    ptr: NonNull<Shared>,
    _fd: OwnedFd,
}

// SAFETY: the mmap is address-independent and lives until Mapping::drop. Moving
// ownership never exposes concurrent access to an Endpoint. Shared access occurs
// only through atomic metadata and the role-owned SPSC regions described below.
unsafe impl Send for Mapping {}

impl Mapping {
    fn shared(&self) -> &Shared {
        // SAFETY: validated, correctly aligned, full-size mapping remains mapped.
        unsafe { self.ptr.as_ref() }
    }
}

impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this Mapping uniquely owns this mapping, not the underlying fd's
        // other mappings. All borrowed ring references have ended before drop.
        unsafe { libc::munmap(self.ptr.as_ptr().cast(), size_of::<Shared>()) };
    }
}

/// One permanently claimed role. Send but intentionally not Sync: each direction
/// has exactly one producer and one consumer, and operations require `&mut self`.
pub struct Endpoint {
    mapping: Mapping,
    role: Role,
    options: Options,
    failed: bool,
    _not_sync: PhantomData<Cell<()>>,
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn validate_options(options: Options) -> io::Result<()> {
    if options.wait_timeout.is_zero()
        || options.wait_timeout > MAX_TIMEOUT
        || options.spin_iterations > MAX_SPINS
    {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid shared transport limits"));
    }
    Ok(())
}

fn map(fd: OwnedFd) -> io::Result<Mapping> {
    let raw = fd.as_raw_fd();
    let mut stat = std::mem::MaybeUninit::<libc::stat>::uninit();
    // SAFETY: fstat initializes stat on success and does not retain its pointer.
    if unsafe { libc::fstat(raw, stat.as_mut_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let stat = unsafe { stat.assume_init() };
    if stat.st_mode & libc::S_IFMT != libc::S_IFREG
        || stat.st_size != size_of::<Shared>() as libc::off_t
        || stat.st_uid != unsafe { libc::geteuid() }
    {
        return Err(invalid("shared transport fd size, type, or owner mismatch"));
    }
    let seals = unsafe { libc::fcntl(raw, libc::F_GET_SEALS) };
    if seals < 0 {
        return Err(io::Error::last_os_error());
    }
    if seals & REQUIRED_SEALS != REQUIRED_SEALS {
        return Err(invalid("shared transport fd lacks fixed-extent seals"));
    }
    // SAFETY: seals prevent truncation after size validation. MAP_SHARED provides
    // page alignment and the writable storage required by the atomic/ring layout.
    let ptr = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            size_of::<Shared>(),
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED,
            raw,
            0,
        )
    };
    if ptr == libc::MAP_FAILED {
        return Err(io::Error::last_os_error());
    }
    Ok(Mapping { ptr: NonNull::new(ptr.cast()).expect("mmap returned null"), _fd: fd })
}

/// Creates a parent endpoint and a CLOEXEC child capability. No child is spawned.
pub fn create() -> io::Result<(Endpoint, OwnedFd)> {
    create_with_options(Options::default())
}

pub fn create_with_options(options: Options) -> io::Result<(Endpoint, OwnedFd)> {
    validate_options(options)?;
    // SAFETY: static NUL-terminated name and supported Linux flags.
    let raw = unsafe {
        libc::syscall(
            libc::SYS_memfd_create,
            c"jr-macro-shm".as_ptr(),
            libc::MFD_CLOEXEC | libc::MFD_ALLOW_SEALING,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    let fd = unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) };
    if unsafe { libc::ftruncate(fd.as_raw_fd(), size_of::<Shared>() as libc::off_t) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd.as_raw_fd(), libc::F_ADD_SEALS, REQUIRED_SEALS) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let child_fd = fd.try_clone()?;
    let mapping = map(fd)?;
    // SAFETY: freshly allocated zero-filled memfd has not been exposed to any
    // peer. Initialize every field before returning its capability to the caller.
    unsafe {
        mapping.ptr.as_ptr().write(Shared {
            magic: MAGIC,
            version: VERSION,
            mapping_size: size_of::<Shared>() as u32,
            capacity: CAPACITY as u32,
            owner_uid: libc::geteuid(),
            creator_pid: libc::getpid(),
            reserved: [0; 1],
            claims: [AtomicU32::new(libc::getpid() as u32), AtomicU32::new(0)],
            closed: [AtomicU32::new(0), AtomicU32::new(0)],
            rings: [Ring::new(), Ring::new()],
        });
    }
    Ok((
        Endpoint { mapping, role: Role::Parent, options, failed: false, _not_sync: PhantomData },
        child_fd,
    ))
}

/// Attaches exactly once. The creator already owns Parent, so attaching Parent to
/// a created transport is rejected. Inherited descriptors remain owned and are
/// closed on any validation failure. Attach before concurrently using the fd.
pub fn attach(fd: OwnedFd, role: Role) -> io::Result<Endpoint> {
    attach_with_options(fd, role, Options::default())
}

pub fn attach_with_options(fd: OwnedFd, role: Role, options: Options) -> io::Result<Endpoint> {
    attach_inner(fd, role, options, None)
}

/// Attaches the child role only after binding the immutable creator PID to an
/// independently authenticated socket peer. The caller must authenticate that
/// peer (for example with SO_PEERCRED) before passing its expected creator PID.
/// The header check is not a substitute for socket authentication: the memfd is
/// writable by its trusted peers, and PID values alone are not credentials.
pub fn attach_child(fd: OwnedFd, expected_creator_pid: libc::pid_t) -> io::Result<Endpoint> {
    attach_child_with_options(fd, expected_creator_pid, Options::default())
}

pub fn attach_child_with_options(
    fd: OwnedFd,
    expected_creator_pid: libc::pid_t,
    options: Options,
) -> io::Result<Endpoint> {
    if expected_creator_pid <= 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "invalid expected creator PID"));
    }
    attach_inner(fd, Role::Child, options, Some(expected_creator_pid))
}

fn attach_inner(
    fd: OwnedFd,
    role: Role,
    options: Options,
    expected_creator_pid: Option<libc::pid_t>,
) -> io::Result<Endpoint> {
    validate_options(options)?;
    let mapping = map(fd)?;
    let shared = mapping.shared();
    if shared.magic != MAGIC
        || shared.version != VERSION
        || shared.mapping_size != size_of::<Shared>() as u32
        || shared.capacity != CAPACITY as u32
        || shared.owner_uid != unsafe { libc::geteuid() }
        || shared.creator_pid <= 0
        || expected_creator_pid.is_some_and(|pid| pid != shared.creator_pid)
        || shared.reserved != [0; 1]
        || shared.claims[0].load(Ordering::Acquire) != shared.creator_pid as u32
    {
        return Err(invalid("shared transport header mismatch"));
    }
    for (index, ring) in shared.rings.iter().enumerate() {
        if shared.closed[index].load(Ordering::Acquire) != 0
            || ring.tail.load(Ordering::Acquire).wrapping_sub(ring.head.load(Ordering::Acquire))
                > CAPACITY as u32
        {
            return Err(invalid("shared transport closed or corrupt cursors"));
        }
    }
    shared.claims[role.index()]
        .compare_exchange(0, unsafe { libc::getpid() as u32 }, Ordering::AcqRel, Ordering::Acquire)
        .map_err(|_| {
            io::Error::new(io::ErrorKind::AlreadyExists, "transport role already claimed")
        })?;
    Ok(Endpoint { mapping, role, options, failed: false, _not_sync: PhantomData })
}

impl Endpoint {
    /// Closes both local directions, wakes both peers, and preserves queued bytes
    /// for the peer to drain. A closed role can never be reattached.
    pub fn close(&mut self) {
        self.failed = true;
        let shared = self.mapping.shared();
        shared.closed[self.role.index()].store(1, Ordering::Release);
        for ring in &shared.rings {
            ring.notify();
        }
    }

    fn wait(&self, ring: &Ring, expected: u32, deadline: Instant) -> io::Result<()> {
        let remaining = deadline.checked_duration_since(Instant::now()).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "shared transport peer made no progress before deadline",
            )
        })?;
        let timeout = libc::timespec {
            tv_sec: remaining.as_secs() as libc::time_t,
            tv_nsec: remaining.subsec_nanos() as libc::c_long,
        };
        ring.sleepers.fetch_add(1, Ordering::SeqCst);
        if ring.event.load(Ordering::SeqCst) != expected {
            ring.sleepers.fetch_sub(1, Ordering::SeqCst);
            return Ok(());
        }
        // SAFETY: aligned shared AtomicU32 stays mapped for this call. FUTEX_WAIT
        // atomically checks expected before sleeping, avoiding notification races.
        let result = unsafe {
            libc::syscall(
                libc::SYS_futex,
                ring.event.as_ptr(),
                libc::FUTEX_WAIT,
                expected,
                &timeout as *const libc::timespec,
            )
        };
        // Capture errno before any further calls. Atomic operations do not alter it.
        let error = if result < 0 { Some(io::Error::last_os_error()) } else { None };
        ring.sleepers.fetch_sub(1, Ordering::SeqCst);
        let Some(error) = error else { return Ok(()) };
        match error.raw_os_error() {
            Some(libc::EAGAIN) | Some(libc::EINTR) => Ok(()),
            Some(libc::ETIMEDOUT) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "shared transport wait expired (peer may have exited)",
            )),
            _ => Err(error),
        }
    }

    fn transfer(&mut self, pointer: *mut u8, len: usize, writing: bool) -> io::Result<usize> {
        if len == 0 {
            return Ok(0);
        }
        if self.failed {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "shared transport is closed"));
        }
        let result = self.transfer_inner(pointer, len, writing);
        if result.is_err() {
            self.close();
        }
        result
    }

    fn transfer_inner(&self, pointer: *mut u8, len: usize, writing: bool) -> io::Result<usize> {
        let shared = self.mapping.shared();
        let own = self.role.index();
        let peer = 1 - own;
        let ring = &shared.rings[if writing { own } else { peer }];
        let deadline = Instant::now() + self.options.wait_timeout;
        let mut spins = self.options.spin_iterations;
        loop {
            // Snapshot before conditions so any intervening close/cursor update
            // either becomes visible below or makes FUTEX_WAIT return EAGAIN.
            let event = ring.event.load(Ordering::Acquire);
            let head = ring.head.load(Ordering::Acquire);
            let tail = ring.tail.load(Ordering::Acquire);
            let used = tail.wrapping_sub(head) as usize;
            if used > CAPACITY {
                return Err(invalid("shared transport ring cursor corruption"));
            }
            let closed = shared.closed[peer].load(Ordering::Acquire) != 0;
            if writing && closed {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "shared transport peer closed",
                ));
            }
            let available = if writing { CAPACITY - used } else { used };
            if available != 0 {
                let cursor = if writing { tail } else { head };
                let offset = cursor as usize % CAPACITY;
                let count = len.min(available).min(CAPACITY - offset);
                // SAFETY: permanent role claims guarantee one producer and one
                // consumer per ring. Acquire of the opposite cursor grants access
                // to exactly this free/initialized range. Release publication below
                // transfers it to the other peer. Bytes never overlap caller memory.
                // Use a raw pointer directly, never an &mut to the entire ring.
                unsafe {
                    let bytes = ring.bytes.get().cast::<u8>().add(offset);
                    if writing {
                        std::ptr::copy_nonoverlapping(pointer.cast_const(), bytes, count);
                    } else {
                        std::ptr::copy_nonoverlapping(bytes, pointer, count);
                    }
                }
                let next = cursor.wrapping_add(count as u32);
                if writing {
                    ring.tail.store(next, Ordering::Release);
                } else {
                    ring.head.store(next, Ordering::Release);
                }
                ring.notify();
                return Ok(count);
            }
            if !writing && closed {
                // Re-read tail after acquiring close, so a final write preceding
                // close cannot be mistaken for EOF using an older tail snapshot.
                if ring.tail.load(Ordering::Acquire) == head {
                    return Ok(0);
                }
                continue;
            }
            if spins != 0 {
                spins -= 1;
                std::hint::spin_loop();
            } else {
                self.wait(ring, event, deadline)?;
            }
        }
    }
}

impl Read for Endpoint {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.transfer(buf.as_mut_ptr(), buf.len(), false)
    }
}

impl Write for Endpoint {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.transfer(buf.as_ptr().cast_mut(), buf.len(), true)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.failed {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "shared transport is closed"))
        } else {
            Ok(()) // write publishes immediately, with no private buffering.
        }
    }
}

impl Drop for Endpoint {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};
    use std::thread;

    fn pair() -> (Endpoint, Endpoint) {
        let (parent, fd) = create().unwrap();
        (parent, attach(fd, Role::Child).unwrap())
    }

    #[test]
    fn framed_large_writes_and_duplex() {
        let (mut parent, mut child) = pair();
        let worker = thread::spawn(move || {
            for size in [0usize, 1, CAPACITY - 1, CAPACITY, CAPACITY * 3 + 17] {
                let mut header = [0; 8];
                child.read_exact(&mut header).unwrap();
                assert_eq!(u64::from_le_bytes(header), size as u64);
                let mut bytes = vec![0; size];
                child.read_exact(&mut bytes).unwrap();
                assert!(bytes.iter().enumerate().all(|(i, b)| *b == i as u8));
                child.write_all(&header).unwrap();
                child.write_all(&bytes).unwrap();
            }
        });
        for size in [0usize, 1, CAPACITY - 1, CAPACITY, CAPACITY * 3 + 17] {
            let bytes: Vec<_> = (0..size).map(|i| i as u8).collect();
            parent.write_all(&(size as u64).to_le_bytes()).unwrap();
            parent.write_all(&bytes).unwrap();
            let mut header = [0; 8];
            parent.read_exact(&mut header).unwrap();
            assert_eq!(u64::from_le_bytes(header), size as u64);
            let mut reply = vec![0; size];
            parent.read_exact(&mut reply).unwrap();
            assert_eq!(reply, bytes);
        }
        worker.join().unwrap();
    }

    #[test]
    fn close_drains_then_eof_and_breaks_writes() {
        let (mut parent, mut child) = pair();
        parent.write_all(b"final").unwrap();
        drop(parent);
        let mut bytes = Vec::new();
        child.read_to_end(&mut bytes).unwrap();
        assert_eq!(bytes, b"final");
        assert_eq!(child.write(b"no").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn close_unblocks_reader_and_full_writer() {
        let (mut parent, child) = pair();
        let reader = thread::spawn(move || parent.read(&mut [0]));
        drop(child);
        assert_eq!(reader.join().unwrap().unwrap(), 0);
        let (mut parent, child) = pair();
        parent.write_all(&[0; CAPACITY]).unwrap();
        let writer = thread::spawn(move || parent.write(&[1]));
        drop(child);
        assert_eq!(writer.join().unwrap().unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn rejects_duplicate_roles_and_bad_fds() {
        let (parent, fd) = create().unwrap();
        assert!(attach(fd.try_clone().unwrap(), Role::Parent).is_err());
        let child = attach(fd.try_clone().unwrap(), Role::Child).unwrap();
        assert!(attach(fd, Role::Child).is_err());
        drop((parent, child));
        assert!(attach(std::fs::File::open("/dev/null").unwrap().into(), Role::Child).is_err());
        let file = tempfile::tempfile().unwrap();
        file.set_len(size_of::<Shared>() as u64).unwrap();
        assert!(attach(file.into(), Role::Child).is_err());
    }

    #[test]
    fn child_creator_binding_precedes_role_claim() {
        let (_parent, fd) = create().unwrap();
        let creator = unsafe { libc::getpid() };
        assert!(attach_child(fd.try_clone().unwrap(), 0).is_err());
        assert!(attach_child(fd.try_clone().unwrap(), creator + 1).is_err());
        // A failed creator check must not consume the one child role claim.
        let _child = attach_child(fd, creator).unwrap();
    }

    #[test]
    fn rejects_malformed_header() {
        let (parent, fd) = create().unwrap();
        // No peer has attached, so modifying the immutable header here has no race.
        unsafe { (*parent.mapping.ptr.as_ptr()).version += 1 };
        assert!(attach(fd, Role::Child).is_err());
    }

    #[test]
    fn timeout_is_bounded_and_terminal() {
        let options = Options { wait_timeout: Duration::from_millis(30), spin_iterations: 8 };
        let (mut parent, _fd) = create_with_options(options).unwrap();
        let start = Instant::now();
        assert_eq!(parent.read(&mut [0]).unwrap_err().kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(parent.write(b"no replay").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    // Executes this same test in a real exec'd child. Only pre_exec's child-local
    // descriptor table has CLOEXEC cleared; the parent's descriptor stays safe.
    #[test]
    fn inherited_child_process() {
        const FD_ENV: &str = "JR_MACRO_SHM_UNIT_TEST_FD";
        if let Some(fd) = std::env::var_os(FD_ENV) {
            let raw: libc::c_int = fd.to_str().unwrap().parse().unwrap();
            let mut child = attach(unsafe { OwnedFd::from_raw_fd(raw) }, Role::Child).unwrap();
            if std::env::var_os("JR_MACRO_SHM_UNIT_TEST_EXIT").is_some() {
                // Intentionally bypass Drop, just like abrupt process death.
                std::process::exit(0);
            }
            let mut message = [0; 4];
            child.read_exact(&mut message).unwrap();
            assert_eq!(&message, b"ping");
            child.write_all(b"pong").unwrap();
            return;
        }
        use std::os::unix::process::CommandExt;
        let (mut parent, fd) = create().unwrap();
        let raw = fd.as_raw_fd();
        let mut command = Command::new(std::env::current_exe().unwrap());
        // libtest names omit the crate-name prefix present in module_path!().
        let module = module_path!().split_once("::").unwrap().1;
        command.arg("--exact").arg(format!("{module}::inherited_child_process"));
        command.env(FD_ENV, raw.to_string()).stdin(Stdio::null());
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(raw, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut process = command.spawn().unwrap();
        drop(fd);
        parent.write_all(b"ping").unwrap();
        let mut reply = [0; 4];
        parent.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"pong");
        assert!(process.wait().unwrap().success());

        // A peer that exits without closing its role must not hang a reader.
        let options = Options { wait_timeout: Duration::from_millis(30), spin_iterations: 0 };
        let (mut parent, fd) = create_with_options(options).unwrap();
        let raw = fd.as_raw_fd();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.arg("--exact").arg(format!("{module}::inherited_child_process"));
        command.env(FD_ENV, raw.to_string()).env("JR_MACRO_SHM_UNIT_TEST_EXIT", "1");
        unsafe {
            command.pre_exec(move || {
                if libc::fcntl(raw, libc::F_SETFD, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        assert!(command.spawn().unwrap().wait().unwrap().success());
        drop(fd);
        assert_eq!(parent.read(&mut [0]).unwrap_err().kind(), io::ErrorKind::TimedOut);
    }
}
