//! Native lifecycle for brokered linker execution, without a helper process.
use std::{
    io,
    path::Path,
    process::{Command, Output},
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn output(command: &mut Command, root: &Path) -> io::Result<Output> {
    use super::link_sandbox_io::{Broker, bootstrap_exec, receive};
    use std::{
        io::Read,
        os::fd::AsRawFd,
        process::{Child, Stdio},
        time::{Duration, Instant},
    };
    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }
    unsafe extern "C" {
        fn poll(fds: *mut PollFd, count: usize, timeout: i32) -> i32;
    }
    struct Guard(Child);
    impl Drop for Guard {
        fn drop(&mut self) {
            if !matches!(self.0.try_wait(), Ok(Some(_))) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
    }
    let broker = Broker::new(root)?;
    let socket = super::link_sandbox::confine(command, root)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Command::spawn waits for its exec error pipe to close. The bootstrap
    // listener must therefore run concurrently with spawn, before exec. No
    // untrusted image has run when this one initial event is continued.
    let (mut child, listener) = std::thread::scope(|scope| {
        let transfer = scope.spawn(move || {
            let listener = receive(&socket)?;
            let pid = bootstrap_exec(&listener)?;
            Ok::<_, io::Error>((listener, pid))
        });
        let child = command.spawn().map(Guard);
        let transfer = transfer
            .join()
            .map_err(|_| io::Error::other("linker bootstrap receiver panicked"))?;
        let child = child?;
        let (listener, pid) = transfer?;
        if pid != child.0.id() {
            return Err(io::Error::other(
                "linker bootstrap process identity mismatch",
            ));
        }
        Ok::<_, io::Error>((child, listener))
    })?;
    let stdout = child
        .0
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing linker stdout pipe"))?;
    let stderr = child
        .0
        .stderr
        .take()
        .ok_or_else(|| io::Error::other("missing linker stderr pipe"))?;
    std::thread::scope(move |scope| {
        let mut child = child;
        fn drain(mut pipe: impl Read) -> io::Result<Vec<u8>> {
            let mut bytes = Vec::new();
            pipe.read_to_end(&mut bytes)?;
            Ok(bytes)
        }
        let out = scope.spawn(move || drain(stdout));
        let err = scope.spawn(move || drain(stderr));
        let deadline = Instant::now() + Duration::from_secs(60);
        let result = (|| {
            let mut requests = 0usize;
            loop {
                if let Some(status) = child.0.try_wait()? {
                    return Ok(status);
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "controlled linker exceeded 60 second execution limit",
                    ));
                }
                let mut fd = PollFd {
                    fd: listener.as_raw_fd(),
                    events: 1,
                    revents: 0,
                };
                // SAFETY: one live, writable native pollfd, bounded timeout.
                if unsafe { poll(&mut fd, 1, 50) } < 0 {
                    let error = io::Error::last_os_error();
                    if error.kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return Err(error);
                }
                if fd.revents & (8 | 32) != 0 {
                    return Err(io::Error::other(
                        "controlled linker notification channel failed",
                    ));
                }
                if fd.revents & 1 != 0 {
                    requests += 1;
                    if requests > 100_000 {
                        return Err(io::Error::other(
                            "controlled linker exceeded 100000 filesystem requests",
                        ));
                    }
                    broker.serve_once(&listener, child.0.id())?;
                }
            }
        })();
        if result.is_err() {
            let _ = child.0.kill();
            let _ = child.0.wait();
        }
        let stdout = out
            .join()
            .map_err(|_| io::Error::other("linker stdout reader panicked"))??;
        let stderr = err
            .join()
            .map_err(|_| io::Error::other("linker stderr reader panicked"))??;
        match result {
            Ok(status) => Ok(Output {
                status,
                stdout,
                stderr,
            }),
            Err(error) => Err(io::Error::new(
                error.kind(),
                format!(
                    "{error}\n{}{}",
                    String::from_utf8_lossy(&stdout),
                    String::from_utf8_lossy(&stderr)
                ),
            )),
        }
    })
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(super) fn output(_: &mut Command, _: &Path) -> io::Result<Output> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "controlled native link sandbox requires Linux x86-64",
    ))
}

/// Allocation-free child bootstrap. The filter permits sendmsg only on this
/// already-connected private socket, which is closed by exec before tool code.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) unsafe fn send_listener(socket: i32, listener: i32) -> io::Result<()> {
    use std::ffi::c_void;
    #[repr(C)]
    struct IoVec {
        base: *mut c_void,
        len: usize,
    }
    #[repr(C)]
    struct Message {
        name: *mut c_void,
        name_len: u32,
        iov: *mut IoVec,
        iov_len: usize,
        control: *mut c_void,
        control_len: usize,
        flags: i32,
    }
    #[repr(C)]
    struct Rights {
        len: usize,
        level: i32,
        kind: i32,
        fd: i32,
        padding: i32,
    }
    unsafe extern "C" {
        fn sendmsg(fd: i32, message: *const Message, flags: i32) -> isize;
    }
    let mut byte = 1u8;
    let mut iov = IoVec {
        base: (&mut byte as *mut u8).cast(),
        len: 1,
    };
    let mut rights = Rights {
        len: 20,
        level: 1,
        kind: 1,
        fd: listener,
        padding: 0,
    };
    let message = Message {
        name: std::ptr::null_mut(),
        name_len: 0,
        iov: &mut iov,
        iov_len: 1,
        control: (&mut rights as *mut Rights).cast(),
        control_len: 24,
        flags: 0,
    };
    // MSG_NOSIGNAL avoids a signal handler in the post-fork child.
    if unsafe { sendmsg(socket, &message, 0x4000) } != 1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}
