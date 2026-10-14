//! Private Linux compiler query binding transport, invisible to argv and environment.
use std::fs::File;
use std::io;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::path::Path;
#[cfg(not(unix))]
type RawFd = i32;

pub const RESERVED_TICKET_FD: RawFd = 198;
pub const PROTOCOL: &str = "jr-rustc-query-binding-v1";
const MAGIC: &[u8; 8] = b"JRQBFD01";
const MAX_TICKET: usize = 16 * 1024;
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub schema: u32,
    pub protocol: String,
    pub run_id: String,
    pub invocation_id: String,
    pub parent_invocation_id: String,
    pub parent_task_id: String,
    pub entry_capture_id: String,
    pub capture_id: String,
    pub receipt_schema: u32,
}
impl Binding {
    fn ids(&self) -> [&str; 6] {
        [
            &self.run_id,
            &self.invocation_id,
            &self.parent_invocation_id,
            &self.parent_task_id,
            &self.entry_capture_id,
            &self.capture_id,
        ]
    }
    pub fn valid(&self) -> bool {
        self.schema == 1
            && self.protocol == PROTOCOL
            && self.receipt_schema == 3
            && self.entry_capture_id != self.capture_id
            && self.ids().iter().all(|s| {
                !s.is_empty()
                    && s.len() <= 128
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
            })
    }
}
#[derive(Debug)]
pub struct PreparedTransport {
    ticket: File,
    directory: File,
}
impl PreparedTransport {
    #[cfg(unix)]
    pub fn ticket_fd(&self) -> RawFd {
        self.ticket.as_raw_fd()
    }
    #[cfg(unix)]
    pub fn directory_fd(&self) -> RawFd {
        self.directory.as_raw_fd()
    }
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid query binding")
}
#[cfg(target_os = "linux")]
mod linux {
    use super::*;
    use std::ffi::CString;
    use std::io::Write;
    use std::os::unix::fs::{FileExt, MetadataExt, OpenOptionsExt};
    unsafe extern "C" {
        fn memfd_create(name: *const std::ffi::c_char, flags: u32) -> i32;
        fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        fn geteuid() -> u32;
        fn dup3(old: i32, new: i32, flags: i32) -> i32;
        fn close(fd: i32) -> i32;
    }
    const SEALS: i32 = 1 | 2 | 4 | 8;
    pub(super) fn private(file: &File) -> bool {
        file.metadata().is_ok_and(|m| {
            m.is_dir() && m.mode() & 0o077 == 0
            // SAFETY: geteuid has no memory preconditions.
            && m.uid() == unsafe { geteuid() }
        })
    }
    fn duplicate(fd: RawFd) -> Option<File> {
        let copy = unsafe { fcntl(fd, 1030, RESERVED_TICKET_FD + 1) };
        (copy >= 0).then(|| unsafe { File::from_raw_fd(copy) })
    }
    impl PreparedTransport {
        /// Child-only after fork: async-signal-safe descriptor operations only.
        /// Reserve atomically before mapping, never overwrite a foreign slot.
        pub unsafe fn map_child(&self) -> bool {
            let ticket = self.ticket_fd();
            let directory = self.directory_fd();
            let directory_flags = unsafe { fcntl(directory, 1) };
            let ticket_flags = unsafe { fcntl(ticket, 1) };
            if directory_flags < 0 || ticket_flags < 0 || directory == RESERVED_TICKET_FD {
                return false;
            }
            let reserved = if ticket == RESERVED_TICKET_FD {
                false
            } else {
                let slot = unsafe { fcntl(ticket, 1030, RESERVED_TICKET_FD) };
                if slot < 0 {
                    return false;
                }
                if slot != RESERVED_TICKET_FD {
                    unsafe { close(slot) };
                    return false;
                }
                true
            };
            if unsafe { fcntl(directory, 2, directory_flags & !1) } < 0 {
                if reserved {
                    unsafe { close(RESERVED_TICKET_FD) };
                }
                return false;
            }
            let mapped = if reserved {
                unsafe { dup3(ticket, RESERVED_TICKET_FD, 0) >= 0 }
            } else {
                unsafe { fcntl(ticket, 2, ticket_flags & !1) >= 0 }
            };
            if !mapped {
                if unsafe { fcntl(directory, 2, directory_flags) } < 0 {
                    // This is our own child-only descriptor. Do not leak it if
                    // restoring CLOEXEC fails after partial materialization.
                    unsafe { close(directory) };
                }
                if reserved {
                    unsafe { close(RESERVED_TICKET_FD) };
                }
                return false;
            }
            true
        }
    }
    pub fn prepare(dir: &Path, binding: &Binding) -> io::Result<PreparedTransport> {
        if !binding.valid() {
            return Err(invalid());
        }
        let mut directory = std::fs::OpenOptions::new()
            .read(true)
            .custom_flags(131072 | 65536 | 524288)
            .open(dir)?;
        if directory.as_raw_fd() == RESERVED_TICKET_FD {
            directory = duplicate(directory.as_raw_fd()).ok_or_else(io::Error::last_os_error)?;
        }
        if !private(&directory) {
            return Err(invalid());
        }
        let name = CString::new("jr-query-binding").unwrap();
        // SAFETY: valid NUL-terminated name, Linux MFD_CLOEXEC | MFD_ALLOW_SEALING.
        let fd = unsafe { memfd_create(name.as_ptr(), 1 | 2) };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: newly allocated fd transferred exactly once.
        let mut ticket = unsafe { File::from_raw_fd(fd) };
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&binding.schema.to_le_bytes());
        bytes.extend_from_slice(&binding.receipt_schema.to_le_bytes());
        bytes.extend_from_slice(&directory.as_raw_fd().to_le_bytes());
        for s in std::iter::once(binding.protocol.as_str()).chain(binding.ids()) {
            bytes.extend_from_slice(&(s.len() as u16).to_le_bytes());
            bytes.extend_from_slice(s.as_bytes());
        }
        if bytes.len() > MAX_TICKET {
            return Err(invalid());
        }
        ticket.write_all(&bytes)?;
        // SAFETY: valid owned memfd, F_ADD_SEALS with all immutable seals.
        if unsafe { fcntl(fd, 1033, SEALS) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(PreparedTransport { ticket, directory })
    }
    /// Borrowed discovery never modifies or consumes original descriptors.
    pub fn receive_discovered(fd: RawFd) -> Option<(Binding, File)> {
        validate_discovered(fd).map(|(binding, directory, _)| (binding, directory))
    }
    /// Process-main-only ownership transfer after complete duplicate validation.
    /// # Safety
    /// The native launch protocol must exclusively transfer valid transport
    /// descriptors to this process, before threads or embedded callers can own
    /// or reassign them. Invalid probes retain their original descriptors.
    pub unsafe fn receive_inherited(fd: RawFd) -> Option<(Binding, File)> {
        let (binding, directory, directory_fd) = validate_discovered(fd)?;
        unsafe {
            close(fd);
            close(directory_fd);
        }
        Some((binding, directory))
    }
    fn validate_discovered(fd: RawFd) -> Option<(Binding, File, RawFd)> {
        if fd < 3 {
            return None;
        }
        // Probe only CLOEXEC duplicates until the whole transport is validated.
        // No foreign descriptor flags, offsets, or ownership are changed.
        let ticket = duplicate(fd)?;
        let metadata = ticket.metadata().ok()?;
        if !metadata.is_file() || metadata.len() < 34 || metadata.len() > MAX_TICKET as u64 {
            return None;
        }
        // F_GET_SEALS rejects ordinary regular files as well as pipes.
        let seals = unsafe { fcntl(ticket.as_raw_fd(), 1034) };
        if seals < 0 || seals & SEALS != SEALS || metadata.nlink() != 0 {
            return None;
        }
        let mut bytes = vec![0; metadata.len() as usize];
        ticket.read_exact_at(&mut bytes, 0).ok()?;
        if bytes.get(..8)? != MAGIC {
            return None;
        }
        let number = |start| {
            Some(u32::from_le_bytes(
                bytes.get(start..start + 4)?.try_into().ok()?,
            ))
        };
        let schema = number(8)?;
        let receipt_schema = number(12)?;
        let directory_fd = number(16)? as i32;
        if directory_fd < 3 || directory_fd == fd {
            return None;
        }
        let mut offset = 20;
        let mut strings = Vec::new();
        for _ in 0..7 {
            let len = u16::from_le_bytes(bytes.get(offset..offset + 2)?.try_into().ok()?) as usize;
            offset += 2;
            if len > 128 {
                return None;
            }
            strings.push(
                std::str::from_utf8(bytes.get(offset..offset + len)?)
                    .ok()?
                    .to_owned(),
            );
            offset += len;
        }
        if offset != bytes.len() {
            return None;
        }
        let mut s = strings.into_iter();
        let binding = Binding {
            schema,
            receipt_schema,
            protocol: s.next()?,
            run_id: s.next()?,
            invocation_id: s.next()?,
            parent_invocation_id: s.next()?,
            parent_task_id: s.next()?,
            entry_capture_id: s.next()?,
            capture_id: s.next()?,
        };
        if !binding.valid() {
            return None;
        }
        let directory = duplicate(directory_fd)?;
        if !private(&directory) {
            return None;
        }
        Some((binding, directory, directory_fd))
    }
}
#[cfg(target_os = "linux")]
pub use linux::{prepare, receive_discovered, receive_inherited};
#[cfg(not(target_os = "linux"))]
pub fn prepare(_: &Path, _: &Binding) -> io::Result<PreparedTransport> {
    Err(invalid())
}
#[cfg(not(target_os = "linux"))]
pub fn receive_discovered(_: RawFd) -> Option<(Binding, File)> {
    None
}

#[cfg(not(target_os = "linux"))]
pub unsafe fn receive_inherited(_: RawFd) -> Option<(Binding, File)> {
    None
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use std::os::fd::IntoRawFd;
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::sync::atomic::{AtomicU64, Ordering};
    static ID: AtomicU64 = AtomicU64::new(1);
    fn directory() -> std::path::PathBuf {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = base.join(format!(
            "query-binding-{}-{}",
            std::process::id(),
            ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn receive(fd: RawFd) -> Option<(Binding, File)> {
        receive_discovered(fd)
    }
    fn binding() -> Binding {
        Binding {
            schema: 1,
            protocol: PROTOCOL.into(),
            receipt_schema: 3,
            run_id: "run".into(),
            invocation_id: "invocation".into(),
            parent_invocation_id: "parent".into(),
            parent_task_id: "task".into(),
            entry_capture_id: "entry".into(),
            capture_id: "query".into(),
        }
    }
    #[test]
    fn reserved_slot_scenarios() {
        // Isolate fixed descriptor tests from the parallel test runner.
        let child_test = std::thread::current()
            .name()
            .unwrap()
            .replace("reserved_slot_scenarios", "reserved_slot_child");
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", &child_test, "--nocapture"])
            .env("JR_BINDING_TEST_CHILD", "1")
            .status()
            .unwrap();
        assert!(status.success());
    }
    #[test]
    fn reserved_slot_child() {
        if std::env::var_os("JR_BINDING_TEST_CHILD").is_none() {
            return;
        }
        unsafe extern "C" {
            fn fcntl(fd: i32, cmd: i32, ...) -> i32;
            fn dup3(old: i32, new: i32, flags: i32) -> i32;
            fn close(fd: i32) -> i32;
            fn pipe(fds: *mut i32) -> i32;
            fn memfd_create(name: *const std::ffi::c_char, flags: u32) -> i32;
        }
        use std::io::{Seek, SeekFrom, Write};
        assert_eq!(unsafe { fcntl(198, 1) }, -1);
        let path = directory();
        let ordinary = File::create(path.join("foreign")).unwrap();
        ordinary.set_len(1024).unwrap();
        let dir = File::open(&path).unwrap();
        let mut pipes = [-1; 2];
        assert_eq!(unsafe { pipe(pipes.as_mut_ptr()) }, 0);
        let reader = unsafe { File::from_raw_fd(pipes[0]) };
        let _writer = unsafe { File::from_raw_fd(pipes[1]) };
        let mut malformed = unsafe { File::from_raw_fd(memfd_create(c"malformed".as_ptr(), 3)) };
        malformed.write_all(&[b'x'; 64]).unwrap();
        malformed.seek(SeekFrom::Start(7)).unwrap();
        assert_eq!(unsafe { fcntl(malformed.as_raw_fd(), 1033, 15) }, 0);
        for foreign in [&ordinary, &reader, &dir, &malformed] {
            for flags in [0, 524288] {
                assert_eq!(unsafe { dup3(foreign.as_raw_fd(), 198, flags) }, 198);
                let before = unsafe { fcntl(198, 1) };
                let status_before = unsafe { fcntl(198, 3) };
                assert!(receive(198).is_none());
                assert_eq!(unsafe { fcntl(198, 1) }, before);
                assert_eq!(unsafe { fcntl(198, 3) }, status_before);
                let prepared = prepare(&path, &binding()).unwrap();
                let directory_flags = unsafe { fcntl(prepared.directory_fd(), 1) };
                assert!(!unsafe { prepared.map_child() });
                assert_eq!(unsafe { fcntl(198, 1) }, before);
                assert_eq!(
                    unsafe { fcntl(prepared.directory_fd(), 1) },
                    directory_flags
                );
                unsafe { close(198) };
            }
        }
        assert_eq!(malformed.stream_position().unwrap(), 7);
        // Even a valid typed ticket is non-owning until directory validation.
        let prepared = prepare(&path, &binding()).unwrap();
        assert_eq!(unsafe { dup3(prepared.ticket_fd(), 198, 0) }, 198);
        let directory_flags = unsafe { fcntl(prepared.directory_fd(), 1) };
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(receive(198).is_none());
        assert_eq!(unsafe { fcntl(198, 1) }, 0);
        assert_eq!(
            unsafe { fcntl(prepared.directory_fd(), 1) },
            directory_flags
        );
        unsafe { close(198) };
        drop(prepared);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        // Exercise the normal low-fd ticket mapping across a real exec.
        use std::os::unix::process::CommandExt;
        let prepared = prepare(&path, &binding()).unwrap();
        assert_ne!(prepared.ticket_fd(), 198);
        let receiver_test = std::thread::current()
            .name()
            .unwrap()
            .replace("reserved_slot_child", "reserved_exec_receiver");
        let mut command = std::process::Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", &receiver_test, "--nocapture"])
            .env("JR_BINDING_TEST_RECEIVER", "1");
        unsafe {
            command.pre_exec(move || {
                prepared.map_child();
                Ok(())
            });
        }
        assert!(command.status().unwrap().success());
        assert_eq!(unsafe { fcntl(198, 1) }, -1);
        // Genuine reserved transport, including directory allocator selecting 198.
        let mut held = Vec::new();
        loop {
            let fd = unsafe { fcntl(ordinary.as_raw_fd(), 1030, 3) };
            assert!(fd >= 0);
            if fd == 198 {
                unsafe { close(fd) };
                break;
            }
            held.push(unsafe { File::from_raw_fd(fd) });
        }
        let prepared = prepare(&path, &binding()).unwrap();
        assert_ne!(prepared.directory_fd(), 198);
        // Relocation frees 198, so memfd_create may own the slot directly.
        assert!(unsafe { prepared.map_child() });
        let PreparedTransport { ticket, directory } = prepared;
        let ticket_fd = ticket.into_raw_fd();
        let directory_fd = directory.into_raw_fd();
        let (received, output) = unsafe { receive_inherited(198) }.unwrap();
        assert_eq!(received, binding());
        assert_eq!(unsafe { fcntl(198, 1) }, -1);
        assert_eq!(unsafe { fcntl(output.as_raw_fd(), 1) } & 1, 1);
        assert_eq!(unsafe { fcntl(directory_fd, 1) }, -1);
        if ticket_fd != 198 {
            unsafe { close(ticket_fd) };
        }
        drop(output);
        drop(held);
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn reserved_exec_receiver() {
        if std::env::var_os("JR_BINDING_TEST_RECEIVER").is_none() {
            return;
        }
        let (received, directory) = unsafe { receive_inherited(RESERVED_TICKET_FD) }.unwrap();
        assert_eq!(received, binding());
        assert!(directory.metadata().unwrap().is_dir());
    }
    #[test]
    fn sealed_round_trip_transfers_owned_descriptors() {
        use std::io::Write;
        let path = directory();
        let mut prepared = prepare(&path, &binding()).unwrap();
        assert!(prepared.ticket.write_all(b"mutation").is_err());
        let PreparedTransport { ticket, directory } = prepared;
        let directory_fd = directory.as_raw_fd();
        unsafe {
            fcntl(ticket.as_raw_fd(), 2, 0);
            fcntl(directory_fd, 2, 0);
        }
        let (received, received_directory) = receive(ticket.as_raw_fd()).unwrap();
        assert_eq!(received, binding());
        assert_ne!(received_directory.as_raw_fd(), directory_fd);
        unsafe extern "C" {
            fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        }
        assert_eq!(unsafe { fcntl(directory_fd, 1) }, 0);
        assert_eq!(unsafe { fcntl(ticket.as_raw_fd(), 1) }, 0);
        assert!(ticket.metadata().is_ok());
        drop(received_directory);
        drop(directory);
        std::fs::remove_dir(path).unwrap();
    }
    #[test]
    fn rejects_pipe_without_reading_or_waiting() {
        unsafe extern "C" {
            fn pipe(fds: *mut i32) -> i32;
        }
        let mut fds = [-1; 2];
        assert_eq!(unsafe { pipe(fds.as_mut_ptr()) }, 0);
        let reader = unsafe { File::from_raw_fd(fds[0]) };
        let writer = unsafe { File::from_raw_fd(fds[1]) };
        assert!(receive(reader.as_raw_fd()).is_none());
        assert!(reader.metadata().unwrap().file_type().is_fifo());
        drop(writer);
    }
    #[test]
    fn rejects_regular_file_and_invalid_fd() {
        let path = directory();
        let file = File::create(path.join("ordinary")).unwrap();
        file.set_len(1024).unwrap();
        assert!(receive(file.as_raw_fd()).is_none());
        assert!(file.metadata().is_ok());
        assert!(receive(-1).is_none());
        assert!(receive(0).is_none());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn rejects_unsealed_bad_magic_and_oversized_memfds() {
        use std::io::Write;
        unsafe extern "C" {
            fn memfd_create(name: *const std::ffi::c_char, flags: u32) -> i32;
            fn fcntl(fd: i32, cmd: i32, ...) -> i32;
        }
        for (size, seal) in [(64, false), (64, true), (MAX_TICKET + 1, true)] {
            let fd = unsafe { memfd_create(c"binding-test".as_ptr(), 3) };
            assert!(fd >= 0);
            let mut file = unsafe { File::from_raw_fd(fd) };
            file.write_all(&vec![b'x'; size]).unwrap();
            if seal {
                assert_eq!(unsafe { fcntl(fd, 1033, 15) }, 0);
            }
            assert!(receive(file.as_raw_fd()).is_none());
            assert!(file.metadata().is_ok());
        }
    }
    #[test]
    fn rejects_nonprivate_directory_and_unsafe_ids() {
        let path = directory();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(prepare(&path, &binding()).is_err());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut bad = binding();
        bad.capture_id = "../escape".into();
        assert!(prepare(&path, &bad).is_err());
        bad.capture_id = "q".repeat(129);
        assert!(prepare(&path, &bad).is_err());
        std::fs::remove_dir(path).unwrap();
    }
    #[test]
    fn rejects_directory_that_becomes_public() {
        let path = directory();
        let PreparedTransport { ticket, directory } = prepare(&path, &binding()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(receive(ticket.as_raw_fd()).is_none());
        assert!(ticket.metadata().is_ok());
        assert!(directory.metadata().is_ok());
        std::fs::remove_dir(path).unwrap();
    }
}
