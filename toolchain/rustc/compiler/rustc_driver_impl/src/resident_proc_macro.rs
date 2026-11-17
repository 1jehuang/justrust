//! Native fresh-process proc-macro endpoint. Never enables resident mode.

use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::process::ExitCode;

/// Validate without taking ownership, so invalid probes do not close foreign fds.
fn validate(fd: RawFd) -> std::io::Result<()> {
    if fd < 3 {
        return Err(std::io::Error::other("helper requires a dedicated socket descriptor"));
    }
    let mut socket_type: libc::c_int = 0;
    let mut len = std::mem::size_of_val(&socket_type) as libc::socklen_t;
    // SAFETY: writable buffers have exactly their advertised lengths. Validation
    // borrows the supplied descriptor and never reads protocol bytes from it.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_TYPE,
            (&mut socket_type as *mut libc::c_int).cast(),
            &mut len,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if len as usize != std::mem::size_of_val(&socket_type) || socket_type != libc::SOCK_STREAM {
        return Err(std::io::Error::other("helper descriptor is not a stream socket"));
    }
    let mut domain: libc::c_int = 0;
    let mut len = std::mem::size_of_val(&domain) as libc::socklen_t;
    // SAFETY: same bounded borrowed-descriptor validation as above.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_DOMAIN,
            (&mut domain as *mut libc::c_int).cast(),
            &mut len,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    if len as usize != std::mem::size_of_val(&domain) || domain != libc::AF_UNIX {
        return Err(std::io::Error::other("helper descriptor is not a Unix socket"));
    }
    // SAFETY: zero is a valid initialization for this C credential structure.
    let mut credentials: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of_val(&credentials) as libc::socklen_t;
    // SAFETY: bounded output buffer, no ownership change or protocol consumption.
    if unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut credentials as *mut libc::ucred).cast(),
            &mut len,
        )
    } != 0
    {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: these process identity functions have no preconditions.
    if len as usize != std::mem::size_of_val(&credentials)
        || credentials.uid != unsafe { libc::geteuid() }
        || credentials.pid != unsafe { libc::getppid() }
    {
        return Err(std::io::Error::other("helper socket must belong to its same-user parent"));
    }
    Ok(())
}

/// # Safety
/// Called once from native main, which exclusively receives the validated fd.
pub unsafe fn run(argument: &str) -> ExitCode {
    let result = (|| {
        let fd: RawFd =
            argument.parse().map_err(|_| std::io::Error::other("invalid helper descriptor"))?;
        validate(fd)?;
        // SAFETY: validation succeeded and native main transfers exclusive ownership.
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        rustc_metadata::resident_proc_macro_helper::run(stream)
    })();
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("jr-rustc proc-macro helper: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsRawFd;

    #[test]
    fn rejects_standard_and_invalid_descriptors() {
        for fd in [-1, 0, 1, 2, i32::MAX] {
            assert!(validate(fd).is_err());
        }
    }

    #[test]
    fn rejects_non_socket_without_closing_it() {
        let file = std::fs::File::open("/dev/null").unwrap();
        assert!(validate(file.as_raw_fd()).is_err());
        assert!(file.metadata().is_ok());
    }

    #[test]
    fn rejects_same_process_peer_without_consuming_or_closing_it() {
        let (mut peer, mut socket) = UnixStream::pair().unwrap();
        assert!(validate(socket.as_raw_fd()).is_err());
        use std::io::{Read, Write};
        peer.write_all(b"x").unwrap();
        let mut byte = [0];
        socket.read_exact(&mut byte).unwrap();
        assert_eq!(&byte, b"x");
    }
}
