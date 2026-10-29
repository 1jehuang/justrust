//! Native open emulation for the controlled linker. Never resume an untrusted
//! pointer-bearing syscall with CONTINUE: inspect a bounded copy, open beneath
//! pinned staging descriptors, and atomically inject the resulting descriptor.
//! Only the initial trusted Command bootstrap exec is continued, before the
//! tool has run any instructions. See bootstrap_exec's lifecycle preconditions.
use std::{
    ffi::{CString, c_int, c_long, c_ulong, c_void},
    fs::{File, OpenOptions},
    io,
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::OpenOptionsExt, net::UnixDatagram},
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
pub(super) fn bootstrap_exec(listener: &OwnedFd) -> io::Result<u32> {
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
    let response = Response {
        id: request.id,
        value: 0,
        error: 0,
        flags: 1,
    };
    if unsafe { ioctl(listener.as_raw_fd(), SEND, &response as *const Response) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(request.pid)
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
    root_fd: File,
    out_fd: File,
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
        Ok(Self {
            root: root.to_owned(),
            root_fd: directory(root)?,
            out_fd: directory(&root.join("out"))?,
        })
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
        let path = copy_path(expected_pid, pointer);
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
            | 0x4000
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
        if !metadata.is_file() && !metadata.is_dir() {
            return respond(fd, request.id, 13);
        }
        if writing && !metadata.is_file() {
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
fn copy_path(pid: u32, pointer: u64) -> Result<Vec<u8>, i32> {
    let mut path = Vec::with_capacity(256);
    while path.len() < 4096 {
        let address = pointer.checked_add(path.len() as u64).ok_or(14)?;
        let mut bytes = [0u8; 256];
        let length = bytes
            .len()
            .min(4096 - (address as usize & 4095))
            .min(4096 - path.len());
        let local = Iovec {
            base: bytes.as_mut_ptr().cast(),
            length,
        };
        let remote = Iovec {
            base: address as usize as *mut c_void,
            length,
        };
        let count = unsafe {
            syscall(
                310,
                pid,
                &local as *const Iovec,
                1usize,
                &remote as *const Iovec,
                1usize,
                0usize,
            )
        };
        if count <= 0 {
            return Err(14);
        }
        let bytes = &bytes[..count as usize];
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
