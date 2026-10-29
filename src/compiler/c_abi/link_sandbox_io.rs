//! Native open emulation for the controlled linker. Never resume an untrusted
//! pointer-bearing syscall with CONTINUE: inspect a bounded copy, open beneath
//! pinned staging descriptors, and atomically inject the resulting descriptor.
//! Only the initial trusted Command bootstrap exec is continued, before the
//! tool has run any instructions. See bootstrap_exec's lifecycle preconditions.
use std::{
    ffi::{CString, c_int, c_long, c_ulong, c_void},
    fs::{File, OpenOptions},
    io::{self, Read},
    os::{
        fd::{AsFd, AsRawFd, FromRawFd, OwnedFd},
        unix::{
            ffi::OsStrExt,
            fs::{FileExt, FileTypeExt, MetadataExt, OpenOptionsExt},
            net::UnixDatagram,
        },
    },
    path::{Component, Path, PathBuf},
};

unsafe extern "C" {
    fn syscall(number: c_long, ...) -> c_long;
    fn ioctl(fd: c_int, operation: c_ulong, ...) -> c_int;
    fn recvmsg(fd: c_int, message: *mut Message, flags: c_int) -> isize;
}
#[repr(C)]
struct Iovec {
    base: *mut c_void,
    length: usize,
}
#[repr(C)]
struct Message {
    name: *mut c_void,
    name_length: u32,
    iov: *mut Iovec,
    iov_length: usize,
    control: *mut c_void,
    control_length: usize,
    flags: c_int,
}
#[repr(C)]
#[derive(Default)]
struct Data {
    number: i32,
    arch: u32,
    ip: u64,
    args: [u64; 6],
}
#[repr(C)]
#[derive(Default)]
struct Notification {
    id: u64,
    pid: u32,
    flags: u32,
    data: Data,
}
#[repr(C)]
struct Response {
    id: u64,
    value: i64,
    error: i32,
    flags: u32,
}
#[repr(C)]
struct AddFd {
    id: u64,
    flags: u32,
    source: u32,
    target: u32,
    target_flags: u32,
}
#[repr(C)]
struct OpenHow {
    flags: u64,
    mode: u64,
    resolve: u64,
}
#[repr(C)]
#[derive(Default)]
struct Sizes {
    notification: u16,
    response: u16,
    data: u16,
}
fn check_sizes() -> io::Result<()> {
    let mut sizes = Sizes::default();
    if unsafe { syscall(317, 3u32, 0u32, &mut sizes as *mut Sizes) } == -1 {
        return Err(io::Error::last_os_error());
    }
    validate_sizes(sizes)
}
fn validate_sizes(sizes: Sizes) -> io::Result<()> {
    if sizes.notification as usize != std::mem::size_of::<Notification>()
        || sizes.response as usize != std::mem::size_of::<Response>()
        || sizes.data as usize != std::mem::size_of::<Data>()
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "unsupported seccomp notification structure sizes",
        ));
    }
    Ok(())
}
const RECV: c_ulong = 0xc0502100;
const SEND: c_ulong = 0xc0182101;
const VALID: c_ulong = 0x40082102;
const ADD: c_ulong = 0x40182103;
const CLOEXEC: u32 = 0x80000;

#[test]
fn controlled_link_notification_abi_rejects_growth_or_truncation() {
    assert!(
        validate_sizes(Sizes {
            notification: 80,
            response: 24,
            data: 64
        })
        .is_ok()
    );
    for sizes in [
        Sizes {
            notification: 88,
            response: 24,
            data: 64,
        },
        Sizes {
            notification: 80,
            response: 32,
            data: 64,
        },
        Sizes {
            notification: 80,
            response: 24,
            data: 72,
        },
        Sizes {
            notification: 72,
            response: 24,
            data: 64,
        },
    ] {
        assert!(validate_sizes(sizes).is_err());
    }
}

/// Consume exactly the initial exec from Command's trusted post-fork bootstrap.
/// The listener is freshly installed and no tool instruction has executed yet.
/// This is NOT a pointer-inspecting authorization: Command and its immutable
/// pre-exec closure are trusted, and this one transition launches their chosen
/// captured executable. Caller must verify the returned pid against Child::id.
/// Never call this on an already running tool's listener, or more than once.
/// Every subsequent exec is denied by serve_once, without CONTINUE.
pub(super) fn bootstrap_exec(listener: &OwnedFd) -> io::Result<(u32, File)> {
    check_sizes()?;
    let mut request = Notification::default();
    loop {
        if unsafe {
            ioctl(
                listener.as_raw_fd(),
                RECV,
                &mut request as *mut Notification,
            )
        } != -1
        {
            break;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(4) {
            return Err(error);
        }
        request = Notification::default();
    }
    if request.data.number != 59
        || request.data.arch != 0xc000003e
        || request.flags != 0
        || request.pid == 0
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected sandbox bootstrap syscall",
        ));
    }
    if !valid(listener.as_raw_fd(), request.id)? {
        return Err(io::Error::other("bootstrap notification canceled"));
    }
    // An open proc directory refers to the original process even after exit
    // and PID reuse. Validate the notification after opening it, before any
    // relative access, so an exit/reuse racing this initial lookup is rejected.
    let procdir = OpenOptions::new()
        .read(true)
        .custom_flags(0x200000 | 0x10000 | CLOEXEC as i32)
        .open(format!("/proc/{}", request.pid))?;
    if !valid(listener.as_raw_fd(), request.id)? {
        return Err(io::Error::other("bootstrap notification canceled"));
    }
    let response = Response {
        id: request.id,
        value: 0,
        error: 0,
        flags: 1,
    };
    if unsafe { ioctl(listener.as_raw_fd(), SEND, &response as *const Response) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok((request.pid, procdir))
}

/// Receive exactly one CLOEXEC listener descriptor. The peer is the trusted
/// pre-exec bootstrap, not the linker. Reject truncated or malformed ancillary.
pub(super) fn receive(socket: &UnixDatagram) -> io::Result<OwnedFd> {
    let mut byte = 0u8;
    let mut iov = Iovec {
        base: (&mut byte as *mut u8).cast(),
        length: 1,
    };
    let mut control = [0usize; 8];
    let mut message = Message {
        name: std::ptr::null_mut(),
        name_length: 0,
        iov: &mut iov,
        iov_length: 1,
        control: control.as_mut_ptr().cast(),
        control_length: std::mem::size_of_val(&control),
        flags: 0,
    };
    let result = unsafe { recvmsg(socket.as_raw_fd(), &mut message, 0x40000000) };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    let mut fds = Vec::new();
    let mut offset = 0usize;
    let mut malformed = result != 1 || message.flags & (8 | 32) != 0;
    while offset + 16 <= message.control_length {
        let pointer = unsafe { control.as_ptr().cast::<u8>().add(offset) };
        let length = unsafe { pointer.cast::<usize>().read_unaligned() };
        if length < 16 || length > message.control_length - offset {
            malformed = true;
            break;
        }
        let level = unsafe { pointer.add(8).cast::<i32>().read_unaligned() };
        let kind = unsafe { pointer.add(12).cast::<i32>().read_unaligned() };
        if level == 1 && kind == 1 {
            if (length - 16) % 4 != 0 {
                malformed = true;
            }
            for index in 0..(length - 16) / 4 {
                let fd = unsafe { pointer.add(16 + index * 4).cast::<i32>().read_unaligned() };
                if fd < 0 {
                    malformed = true;
                } else {
                    // SAFETY: SCM_RIGHTS has transferred ownership of this fd.
                    fds.push(unsafe { OwnedFd::from_raw_fd(fd) });
                }
            }
        } else {
            malformed = true;
        }
        offset += (length + 7) & !7;
    }
    if malformed || fds.len() != 1 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid sandbox listener transfer",
        ));
    }
    Ok(fds.pop().unwrap())
}

pub(super) struct Broker {
    root: PathBuf,
    real_root: PathBuf,
    root_fd: File,
    out_fd: File,
    procdir: Option<File>,
    memory: Option<File>,
    stdio: Vec<((u64, u64), u64)>,
    null: (u64, u64),
}
impl Broker {
    pub(super) fn new(root: &Path) -> io::Result<Self> {
        check_sizes()?;
        if !root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "staging root must be absolute",
            ));
        }
        let directory = |path: &Path| {
            OpenOptions::new()
                .read(true)
                .custom_flags(0x200000 | 0x10000 | 0x20000)
                .open(path)
        };
        let null = std::fs::metadata("/dev/null")?;
        if !null.file_type().is_char_device() || null.rdev() != 0x103 {
            return Err(io::Error::other("sandbox stdin requires the null device"));
        }
        Ok(Self {
            root: root.to_owned(),
            real_root: std::fs::canonicalize(root)?,
            root_fd: directory(root)?,
            out_fd: directory(&root.join("out"))?,
            procdir: None,
            memory: None,
            stdio: Vec::new(),
            null: (null.dev(), null.ino()),
        })
    }

    /// Bind only after initial exec succeeds. The proc directory pins task
    /// identity, and the mem file pins its post-exec mm. No later operation
    /// addresses a task by a reusable numeric PID.
    pub(super) fn bind_process(&mut self, procdir: File) -> io::Result<()> {
        if self.procdir.is_some() {
            return Err(io::Error::other("broker process already bound"));
        }
        let result = unsafe {
            syscall(
                257,
                procdir.as_raw_fd(),
                c"mem".as_ptr(),
                2u32 | CLOEXEC,
                0u32,
            )
        };
        if result == -1 {
            return Err(io::Error::last_os_error());
        }
        self.memory = Some(unsafe { File::from_raw_fd(result as i32) });
        self.procdir = Some(procdir);
        Ok(())
    }

    pub(super) fn set_stdio(&mut self, stdout: &impl AsFd, stderr: &impl AsFd) -> io::Result<()> {
        let metadata = |fd: std::os::fd::BorrowedFd<'_>| -> io::Result<std::fs::Metadata> {
            File::from(fd.try_clone_to_owned()?).metadata()
        };
        let output = metadata(stdout.as_fd())?;
        let error = metadata(stderr.as_fd())?;
        if !output.file_type().is_fifo() || !error.file_type().is_fifo() {
            return Err(io::Error::other("sandbox stdio must use private pipes"));
        }
        self.stdio = vec![
            ((output.dev(), output.ino()), 1),
            ((error.dev(), error.ino()), 2),
        ];
        Ok(())
    }

    fn mappings(&self) -> io::Result<Vec<Mapping>> {
        let procdir = self
            .procdir
            .as_ref()
            .ok_or_else(|| io::Error::other("broker process not bound"))?;
        let maps = open_relative(procdir, c"maps", CLOEXEC)?;
        let mut bytes = Vec::new();
        maps.take(4 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > 4 * 1024 * 1024 {
            return Err(io::Error::other("sandbox memory map exceeds 4 MiB"));
        }
        parse_mappings(&bytes)
    }

    fn stat_descriptor(&self, listener: i32, request: &Notification) -> io::Result<()> {
        if !valid(listener, request.id)? {
            return Ok(());
        }
        let descriptor = request.data.args[0] as i32;
        if descriptor < 0 {
            return respond(listener, request.id, 9);
        }
        let procdir = self
            .procdir
            .as_ref()
            .ok_or_else(|| io::Error::other("broker process not bound"))?;
        let path = CString::new(format!("fd/{descriptor}")).unwrap();
        // O_PATH follows the pinned proc magiclink to duplicate only the
        // descriptor's inode reference, never opening device/pipe content.
        let source = match open_relative(procdir, &path, 0x200000 | CLOEXEC) {
            Ok(file) => file,
            Err(error) if matches!(error.raw_os_error(), Some(2 | 9)) => {
                return respond(listener, request.id, 9);
            }
            Err(error) => return Err(error),
        };
        let metadata = source.metadata()?;
        let key = (metadata.dev(), metadata.ino());
        let (identity, mode, size) = if metadata.is_file() {
            let mut target = [0u8; 4096];
            let length = unsafe {
                syscall(
                    267,
                    procdir.as_raw_fd(),
                    path.as_ptr(),
                    target.as_mut_ptr(),
                    target.len(),
                )
            };
            if length < 0 {
                return Err(io::Error::last_os_error());
            }
            if length as usize == target.len() {
                return respond(listener, request.id, 13);
            }
            let target = Path::new(std::ffi::OsStr::from_bytes(&target[..length as usize]));
            let relative = match target.strip_prefix(&self.real_root) {
                Ok(path)
                    if path
                        .components()
                        .all(|part| matches!(part, Component::Normal(_) | Component::CurDir)) =>
                {
                    path
                }
                _ => return respond(listener, request.id, 13),
            };
            let output = relative.starts_with("out");
            let mode = 0o100000 | if output { 0o600 } else { 0o444 };
            let size = metadata.len();
            (
                virtual_identity(b"path", relative.as_os_str().as_bytes()),
                mode,
                size,
            )
        } else if metadata.file_type().is_fifo() {
            match self.stdio.iter().find(|(candidate, _)| *candidate == key) {
                Some((_, tag)) => (virtual_identity(b"pipe", &tag.to_le_bytes()), 0o010600, 0),
                None => return respond(listener, request.id, 13),
            }
        } else if metadata.file_type().is_char_device()
            && key == self.null
            && metadata.rdev() == 0x103
        {
            (virtual_identity(b"null", b"stdin"), 0o020666, 0)
        } else {
            return respond(listener, request.id, 13);
        };
        let bytes = canonical_stat(identity, mode, size);
        let destination = request.data.args[1];
        let mappings = self.mappings()?;
        if !permitted_range(&mappings, destination, bytes.len(), true) {
            return respond(listener, request.id, 14);
        }
        if !valid(listener, request.id)? {
            return Ok(());
        }
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| io::Error::other("broker process not bound"))?;
        let mut written = 0usize;
        while written < bytes.len() {
            // Unlike numeric process_vm_writev, this fd cannot target a reused
            // PID. Writable VMA checks above preserve copy_to_user semantics
            // despite proc mem's FOLL_FORCE behavior. The child is single-
            // threaded and blocked, so its own maps/fd table cannot race us.
            match memory.write_at(&bytes[written..], destination + written as u64) {
                Ok(0) | Err(_) => return respond(listener, request.id, 14),
                Ok(count) => written += count,
            }
        }
        respond(listener, request.id, 0)
    }

    /// Caller polls the listener and supervises child lifetime. A canceled
    /// notification is normal, all other transport failures abort the child.
    pub(super) fn serve_once(&self, listener: &OwnedFd, expected_pid: u32) -> io::Result<()> {
        let fd = listener.as_raw_fd();
        let mut request = Notification::default();
        if unsafe { ioctl(fd, RECV, &mut request as *mut Notification) } == -1 {
            let error = io::Error::last_os_error();
            return if matches!(error.raw_os_error(), Some(2 | 4)) {
                Ok(())
            } else {
                Err(error)
            };
        }
        if request.pid != expected_pid || request.flags != 0 || request.data.arch != 0xc000003e {
            return respond(fd, request.id, 1);
        }
        if request.data.number == 5 {
            return self.stat_descriptor(fd, &request);
        }
        let (pointer, flags, mode, dirfd) = match request.data.number {
            59 => return respond(fd, request.id, 13),
            2 => (
                request.data.args[0],
                request.data.args[1] as u32,
                request.data.args[2] as u32,
                -100,
            ),
            257 => (
                request.data.args[1],
                request.data.args[2] as u32,
                request.data.args[3] as u32,
                request.data.args[0] as i32,
            ),
            _ => return respond(fd, request.id, 1),
        };
        if !valid(fd, request.id)? {
            return Ok(());
        }
        let memory = self
            .memory
            .as_ref()
            .ok_or_else(|| io::Error::other("broker process not bound"))?;
        let mappings = self.mappings()?;
        let path = copy_path(memory, &mappings, pointer);
        if !valid(fd, request.id)? {
            return Ok(());
        }
        let path = match path {
            Ok(path) => path,
            Err(errno) => return respond(fd, request.id, errno),
        };
        let path = Path::new(std::ffi::OsStr::from_bytes(&path));
        let relative = if path.is_absolute() {
            match path.strip_prefix(&self.root) {
                Ok(path) => path,
                Err(_) => return respond(fd, request.id, 13),
            }
        } else {
            if dirfd != -100 {
                return respond(fd, request.id, 13);
            }
            path
        };
        let mut normalized = PathBuf::new();
        for component in relative.components() {
            match component {
                Component::Normal(part) => normalized.push(part),
                Component::CurDir => {}
                _ => return respond(fd, request.id, 13),
            }
        }
        // Permit only known ordinary file flags. No O_PATH, O_TMPFILE,
        // O_ASYNC, special lookup semantics, or unknown future flag grants.
        const FLAGS: u32 = 3
            | 0x40
            | 0x80
            | 0x100
            | 0x200
            | 0x400
            | 0x800
            | 0x1000
            | 0x8000
            | 0x10000
            | 0x20000
            | CLOEXEC
            | 0x100000;
        if flags & !FLAGS != 0 || flags & 3 == 3 || (flags & 0x40 != 0 && mode & !0o777 != 0) {
            return respond(fd, request.id, 13);
        }
        let writing = flags & (3 | 0x40 | 0x200 | 0x400) != 0;
        let (base, relative) = if writing {
            match normalized.strip_prefix("out") {
                Ok(path) if !path.as_os_str().is_empty() => (&self.out_fd, path),
                _ => return respond(fd, request.id, 13),
            }
        } else {
            (&self.root_fd, normalized.as_path())
        };
        let bytes = if relative.as_os_str().is_empty() {
            b".".as_slice()
        } else {
            relative.as_os_str().as_bytes()
        };
        let path = CString::new(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "NUL in copied path"))?;
        // NONBLOCK prevents a malicious FIFO from hanging the broker. No
        // symlink is ever followed, including final output or /proc magiclinks.
        let how = OpenHow {
            flags: (flags | CLOEXEC | 0x800) as u64,
            mode: if flags & 0x40 != 0 { mode as u64 } else { 0 },
            resolve: 0x08 | 0x04,
        };
        let result = unsafe {
            syscall(
                437,
                base.as_raw_fd(),
                path.as_ptr(),
                &how as *const OpenHow,
                std::mem::size_of::<OpenHow>(),
            )
        };
        if result == -1 {
            return respond(
                fd,
                request.id,
                io::Error::last_os_error().raw_os_error().unwrap_or(5),
            );
        }
        let file = unsafe { File::from_raw_fd(result as i32) };
        let metadata = file.metadata()?;
        // Directory descriptors expose physical inode/allocation state through
        // getdents and lseek. The controlled linker uses explicit file paths.
        if !metadata.is_file() {
            return respond(fd, request.id, 13);
        }
        if !valid(fd, request.id)? {
            return Ok(());
        }
        let add = AddFd {
            id: request.id,
            flags: 2,
            source: file.as_raw_fd() as u32,
            target: 0,
            target_flags: flags & CLOEXEC,
        };
        if unsafe { ioctl(fd, ADD, &add as *const AddFd) } == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(2) {
                return Ok(());
            }
            return Err(error);
        }
        Ok(())
    }
}
fn open_relative(parent: &File, path: &std::ffi::CStr, flags: u32) -> io::Result<File> {
    let result = unsafe { syscall(257, parent.as_raw_fd(), path.as_ptr(), flags, 0u32) };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(unsafe { File::from_raw_fd(result as i32) })
    }
}

fn virtual_identity(kind: &[u8], name: &[u8]) -> [u8; 16] {
    let mut hash = blake3::Hasher::new();
    hash.update(b"jr-controlled-stat-v1\0");
    hash.update(kind);
    hash.update(&[0]);
    hash.update(name);
    hash.finalize().as_bytes()[..16].try_into().unwrap()
}

fn canonical_stat(identity: [u8; 16], mode: u32, size: u64) -> [u8; 144] {
    let mut bytes = [0u8; 144];
    bytes[..16].copy_from_slice(&identity); // virtual dev + ino, 128 bits
    bytes[16..24].copy_from_slice(&1u64.to_le_bytes()); // nlink
    bytes[24..28].copy_from_slice(&mode.to_le_bytes());
    bytes[48..56].copy_from_slice(&size.to_le_bytes());
    bytes[56..64].copy_from_slice(&4096u64.to_le_bytes());
    bytes[64..72].copy_from_slice(&size.div_ceil(512).to_le_bytes());
    // uid, gid, padding, rdev, all timestamps and reserved words stay zero.
    bytes
}

#[derive(Debug)]
struct Mapping {
    start: u64,
    end: u64,
    read: bool,
    write: bool,
}
fn parse_mappings(bytes: &[u8]) -> io::Result<Vec<Mapping>> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidData, "invalid pinned memory map");
    let mut maps = Vec::new();
    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let mut fields = line
            .split(|byte| byte.is_ascii_whitespace())
            .filter(|field| !field.is_empty());
        let range =
            std::str::from_utf8(fields.next().ok_or_else(invalid)?).map_err(|_| invalid())?;
        let (start, end) = range.split_once('-').ok_or_else(invalid)?;
        let start = u64::from_str_radix(start, 16).map_err(|_| invalid())?;
        let end = u64::from_str_radix(end, 16).map_err(|_| invalid())?;
        let permissions = fields.next().ok_or_else(invalid)?;
        if start >= end
            || permissions.len() != 4
            || maps.last().is_some_and(|map: &Mapping| map.end > start)
        {
            return Err(invalid());
        }
        maps.push(Mapping {
            start,
            end,
            read: permissions[0] == b'r',
            write: permissions[1] == b'w',
        });
    }
    Ok(maps)
}
fn permitted_range(maps: &[Mapping], address: u64, length: usize, writing: bool) -> bool {
    let Some(end) = address.checked_add(length as u64) else {
        return false;
    };
    let mut cursor = address;
    for map in maps {
        if map.end <= cursor {
            continue;
        }
        if map.start > cursor || if writing { !map.write } else { !map.read } {
            return false;
        }
        cursor = map.end.min(end);
        if cursor == end {
            return true;
        }
    }
    false
}

#[test]
fn controlled_link_canonical_stat_layout_and_mapping_permissions() {
    let identity = virtual_identity(b"path", b"inputs/native.o");
    assert_ne!(identity, virtual_identity(b"path", b"inputs/other.o"));
    let bytes = canonical_stat(identity, 0o100444, 513);
    assert_eq!(&bytes[..16], &identity);
    assert_eq!(&bytes[28..48], &[0; 20]);
    assert_eq!(&bytes[48..56], &513u64.to_le_bytes());
    assert_eq!(&bytes[64..72], &2u64.to_le_bytes());
    assert_eq!(&bytes[72..], &[0; 72]);
    let maps = parse_mappings(
        b"1000-2000 r--p 0 00:00 0\n2000-3000 rw-p 0 00:00 0\n3000-4000 ---p 0 00:00 0\n",
    )
    .unwrap();
    assert!(permitted_range(&maps, 0x1ff0, 32, false));
    assert!(!permitted_range(&maps, 0x1ff0, 32, true));
    assert!(permitted_range(&maps, 0x2000, 144, true));
    assert!(!permitted_range(&maps, 0x2ff0, 32, false));
    assert!(!permitted_range(&maps, u64::MAX - 4, 144, true));
    assert!(parse_mappings(b"2000-1000 rw-p 0 00:00 0\n").is_err());
}

fn valid(fd: i32, id: u64) -> io::Result<bool> {
    if unsafe { ioctl(fd, VALID, &id as *const u64) } != -1 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(2) {
        Ok(false)
    } else {
        Err(error)
    }
}
fn respond(fd: i32, id: u64, errno: i32) -> io::Result<()> {
    let response = Response {
        id,
        value: 0,
        error: -errno.abs(),
        flags: 0,
    };
    if unsafe { ioctl(fd, SEND, &response as *const Response) } != -1 {
        return Ok(());
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() == Some(2) {
        Ok(())
    } else {
        Err(error)
    }
}
#[test]
fn controlled_link_pinned_memory_reads_are_bounded() {
    let memory = File::open("/proc/self/mem").unwrap();
    let unterminated = vec![b'x'; 4096];
    let maps = parse_mappings(&std::fs::read("/proc/self/maps").unwrap()).unwrap();
    assert_eq!(
        copy_path(&memory, &maps, c"frozen".as_ptr() as u64).unwrap(),
        b"frozen"
    );
    assert_eq!(copy_path(&memory, &maps, c"".as_ptr() as u64), Err(2));
    assert_eq!(copy_path(&memory, &maps, 0), Err(14));
    assert_eq!(
        copy_path(&memory, &maps, unterminated.as_ptr() as u64),
        Err(36)
    );
}

fn copy_path(memory: &File, maps: &[Mapping], pointer: u64) -> Result<Vec<u8>, i32> {
    let mut path = Vec::with_capacity(256);
    while path.len() < 4096 {
        let address = pointer.checked_add(path.len() as u64).ok_or(14)?;
        let readable_end = maps
            .iter()
            .find(|map| map.read && map.start <= address && address < map.end)
            .ok_or(14)?
            .end;
        let mut bytes = [0u8; 256];
        let length = bytes
            .len()
            .min(4096 - (address as usize & 4095))
            .min((readable_end - address) as usize)
            .min(4096 - path.len());
        let count = memory
            .read_at(&mut bytes[..length], address)
            .map_err(|_| 14)?;
        if count == 0 {
            return Err(14);
        }
        let bytes = &bytes[..count];
        if let Some(end) = bytes.iter().position(|byte| *byte == 0) {
            path.extend_from_slice(&bytes[..end]);
            if path.is_empty() {
                return Err(2);
            }
            return Ok(path);
        }
        path.extend_from_slice(bytes);
    }
    Err(36)
}
