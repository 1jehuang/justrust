//! Request-owned parent for the fresh native proc-macro helper.
//!
//! This is not an admission policy. Callers must keep this owner scoped to one
//! compiler request and must not route nested expansions over this sequential
//! connection. Failed operations are never retried in this or another process.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::Command;

use rustc_data_structures::jobserver::resident::{RequestChild, RequestClient};
use rustc_data_structures::resident_macro_transport::{self as transport, Outcome};

use crate::resident_proc_macro_helper::{decode_loaded, encode_expand, encode_load};

pub struct MacroHelper {
    session: Option<transport::Session<HelperTransport>>,
    child: Option<RequestChild>,
}

enum HelperTransport {
    Unix(UnixStream),
    #[cfg(target_arch = "x86_64")]
    Shared {
        endpoint: rustc_data_structures::resident_macro_shm::Endpoint,
        // Authentication remains mandatory, even though RPC uses the memfd.
        _socket: UnixStream,
    },
}

impl Read for HelperTransport {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        match self {
            Self::Unix(socket) => socket.read(bytes),
            #[cfg(target_arch = "x86_64")]
            Self::Shared { endpoint, .. } => endpoint.read(bytes),
        }
    }
}

impl Write for HelperTransport {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match self {
            Self::Unix(socket) => socket.write(bytes),
            #[cfg(target_arch = "x86_64")]
            Self::Shared { endpoint, .. } => endpoint.write(bytes),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            Self::Unix(socket) => socket.flush(),
            #[cfg(target_arch = "x86_64")]
            Self::Shared { endpoint, .. } => endpoint.flush(),
        }
    }
}

fn shm_enabled(value: Option<&std::ffi::OsStr>) -> io::Result<bool> {
    match value {
        None => Ok(false),
        Some(value) if value == "0" => Ok(false),
        Some(value) if value == "1" => Ok(true),
        Some(_) => Err(io::Error::other("JUSTRUST_CARGO_RESIDENT_MACRO_SHM must be 0 or 1")),
    }
}

impl MacroHelper {
    /// Starts a fresh helper with the current request's real jobserver, not the
    /// long-lived worker's environment descriptor numbers. No standard stream is
    /// used for RPC or redirected away from the compiler request.
    pub fn spawn(executable: &Path, scheduler: &RequestClient) -> io::Result<Self> {
        let use_shm =
            shm_enabled(std::env::var_os("JUSTRUST_CARGO_RESIDENT_MACRO_SHM").as_deref())?;
        #[cfg(not(target_arch = "x86_64"))]
        if use_shm {
            return Err(io::Error::other("shared proc-macro transport requires Linux x86-64"));
        }
        #[cfg(target_arch = "x86_64")]
        let shared = if use_shm {
            let options = rustc_data_structures::resident_macro_shm::Options::with_spin_argument(
                std::env::var_os("JUSTRUST_CARGO_RESIDENT_MACRO_SPINS").as_deref(),
            )?;
            Some(rustc_data_structures::resident_macro_shm::create_with_options(options)?)
        } else {
            None
        };
        let (parent, endpoint) = UnixStream::pair()?;
        let fd = endpoint.as_raw_fd();
        #[cfg(target_arch = "x86_64")]
        let shm_fd = shared.as_ref().map(|(_, fd)| fd.as_raw_fd());
        #[cfg(not(target_arch = "x86_64"))]
        let shm_fd: Option<std::os::fd::RawFd> = None;
        let mut command = Command::new(executable);
        command.arg("--jr-proc-macro-helper").arg(fd.to_string());
        if let Some(shm_fd) = shm_fd {
            command.arg(shm_fd.to_string());
        }
        // SAFETY: the child closure only performs async-signal-safe fcntl. The
        // endpoint stays owned and open until spawn returns. All other inherited
        // capability setup is owned by RequestClient::spawn_child.
        unsafe {
            command.pre_exec(move || {
                for fd in [Some(fd), shm_fd].into_iter().flatten() {
                    let flags = libc::fcntl(fd, libc::F_GETFD);
                    if flags == -1
                        || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let child = scheduler.spawn_child(command)?;
        drop(endpoint);
        let parent = HelperTransport::Unix(parent);
        #[cfg(target_arch = "x86_64")]
        let parent = match (shared, parent) {
            (Some((endpoint, child_fd)), HelperTransport::Unix(socket)) => {
                drop(child_fd);
                HelperTransport::Shared { endpoint, _socket: socket }
            }
            (None, parent) => parent,
            _ => unreachable!(),
        };
        Ok(Self { session: Some(transport::Session::new(parent)), child: Some(child) })
    }

    fn invoke(
        &mut self,
        payload: Vec<u8>,
        callback: impl FnMut(Vec<u8>) -> io::Result<Vec<u8>>,
    ) -> io::Result<Vec<u8>> {
        let result = self
            .session
            .as_mut()
            .ok_or_else(|| io::Error::other("proc-macro helper is poisoned"))?
            .invoke(payload, callback);
        match result {
            Ok(Outcome::Complete(bytes)) => Ok(bytes),
            other => {
                // Closing the socket stops a helper waiting for a callback. The
                // tracked child is killed and reaped before this request owner
                // can be released. In particular, never restart after Invoke.
                self.session.take();
                self.child.take();
                match other {
                    Err(error) => Err(error),
                    Ok(Outcome::Failure(message)) => {
                        Err(io::Error::other(String::from_utf8_lossy(&message).into_owned()))
                    }
                    Ok(Outcome::Shutdown) => Err(io::Error::other("proc-macro helper shut down")),
                    Ok(Outcome::Complete(_)) => unreachable!(),
                }
            }
        }
    }

    pub fn load(&mut self, path: &Path, symbol: &str) -> io::Result<(u32, u32)> {
        let payload = encode_load(path, symbol)?;
        let bytes = self.invoke(payload, |_| {
            Err(io::Error::other("unexpected callback during proc-macro Load"))
        })?;
        match decode_loaded(&bytes) {
            Ok(loaded) => Ok(loaded),
            Err(error) => {
                self.session.take();
                self.child.take();
                Err(error)
            }
        }
    }

    pub fn expand(
        &mut self,
        library: u32,
        client: u32,
        show_panics: bool,
        input: &[u8],
        callback: impl FnMut(Vec<u8>) -> io::Result<Vec<u8>>,
    ) -> io::Result<Vec<u8>> {
        self.invoke(encode_expand(library, client, show_panics, input)?, callback)
    }
}

impl Drop for MacroHelper {
    fn drop(&mut self) {
        // Do not wait indefinitely for a user macro's destructor or background
        // thread. Fresh rustc request teardown owns this helper's lifetime.
        self.session.take();
        self.child.take();
    }
}

#[cfg(test)]
pub(crate) static REQUEST_MACRO_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::REQUEST_MACRO_TEST_LOCK as LOCK;
    use super::*;
    use rustc_data_structures::jobserver::resident::RequestLease;

    #[test]
    fn shared_transport_is_strictly_opt_in() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        assert!(!shm_enabled(None).unwrap());
        assert!(!shm_enabled(Some(OsStr::new("0"))).unwrap());
        assert!(shm_enabled(Some(OsStr::new("1"))).unwrap());
        for invalid in ["", "true", "false", "01", " 1", "1 "] {
            assert!(shm_enabled(Some(OsStr::new(invalid))).is_err());
        }
        assert!(shm_enabled(Some(OsStr::from_bytes(&[0xff]))).is_err());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn shared_transport_callbacks_and_terminal_failure_do_not_replay() {
        let _lock = LOCK.lock().unwrap();
        let lease = RequestLease::begin_serial().unwrap();
        let scheduler = lease.client();
        let mut command = Command::new("/bin/sleep");
        command.arg("60");
        let child = scheduler.spawn_child(command).unwrap();
        drop(scheduler);
        let (socket, _peer_socket) = UnixStream::pair().unwrap();
        let (endpoint, fd) = rustc_data_structures::resident_macro_shm::create().unwrap();
        let remote =
            rustc_data_structures::resident_macro_shm::attach_child(fd, unsafe { libc::getpid() })
                .unwrap();
        let peer = std::thread::spawn(move || {
            let mut session = transport::Session::new(remote);
            session
                .respond(|_, callback| {
                    assert_eq!(callback(vec![7])?, vec![8]);
                    Ok(vec![9])
                })
                .unwrap();
            session.respond(|_, _| Err(io::Error::other("terminal shared failure"))).unwrap();
        });
        let mut helper = MacroHelper {
            session: Some(transport::Session::new(HelperTransport::Shared {
                endpoint,
                _socket: socket,
            })),
            child: Some(child),
        };
        assert_eq!(
            helper
                .expand(0, 0, false, &[], |bytes| {
                    assert_eq!(bytes, vec![7]);
                    Ok(vec![8])
                })
                .unwrap(),
            vec![9]
        );
        assert!(helper.load(Path::new("/fixture.so"), "symbol").is_err());
        assert!(helper.session.is_none());
        assert!(helper.child.is_none());
        assert!(helper.load(Path::new("/fixture.so"), "symbol").is_err());
        peer.join().unwrap();
        lease.finish().unwrap();
    }

    #[test]
    #[ignore = "requires an explicitly supplied matching native helper and proc-macro library"]
    fn native_helper_load_and_terminal_cleanup() {
        let _lock = LOCK.lock().unwrap();
        let executable = std::env::var_os("JUSTRUST_TEST_RESIDENT_HELPER").unwrap();
        let library = std::env::var_os("JUSTRUST_TEST_RESIDENT_MACRO_LIBRARY").unwrap();
        let symbol = std::env::var("JUSTRUST_TEST_RESIDENT_MACRO_SYMBOL").unwrap();
        for _ in 0..2 {
            let lease = RequestLease::begin_serial().unwrap();
            let scheduler = lease.client();
            let mut helper = MacroHelper::spawn(Path::new(&executable), &scheduler).unwrap();
            drop(scheduler);
            let loaded = helper.load(Path::new(&library), &symbol).unwrap();
            assert_eq!(loaded.0, 0);
            assert!(loaded.1 > 0);
            assert_eq!(helper.load(Path::new(&library), &symbol).unwrap(), loaded);
            assert!(
                helper
                    .expand(0, loaded.1, false, &[], |_| {
                        panic!("invalid client must never issue callbacks")
                    })
                    .is_err()
            );
            assert!(helper.session.is_none());
            assert!(helper.child.is_none());
            assert!(helper.load(Path::new(&library), &symbol).is_err());
            // Failure cleanup has already reaped the child. Keeping the now
            // poisoned facade alive must not keep scheduler ownership active.
            lease.finish().unwrap();
        }
    }

    #[test]
    fn parent_load_expand_callback_and_cleanup_are_request_owned() {
        let _lock = LOCK.lock().unwrap();
        let lease = RequestLease::begin_serial().unwrap();
        let scheduler = lease.client();
        let mut command = Command::new("/bin/sleep");
        command.arg("60");
        let child = scheduler.spawn_child(command).unwrap();
        drop(scheduler);
        let (parent, remote) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            let mut session = transport::Session::new(remote);
            session
                .respond(|payload, _| {
                    assert_eq!(payload[0], 1);
                    let mut reply = 0u32.to_le_bytes().to_vec();
                    reply.extend_from_slice(&1u32.to_le_bytes());
                    Ok(reply)
                })
                .unwrap();
            session
                .respond(|payload, callback| {
                    assert_eq!(payload[0], 2);
                    assert_eq!(callback(vec![7])?, vec![8]);
                    Ok(vec![9])
                })
                .unwrap();
        });
        let mut helper = MacroHelper {
            session: Some(transport::Session::new(HelperTransport::Unix(parent))),
            child: Some(child),
        };
        assert_eq!(helper.load(Path::new("/fixture.so"), "symbol").unwrap(), (0, 1));
        assert_eq!(
            helper
                .expand(0, 0, false, &[3], |bytes| {
                    assert_eq!(bytes, vec![7]);
                    Ok(vec![8])
                })
                .unwrap(),
            vec![9]
        );
        peer.join().unwrap();
        drop(helper);
        lease.finish().unwrap();
    }

    #[test]
    fn parent_terminal_failure_poisoned_without_replay() {
        let _lock = LOCK.lock().unwrap();
        let lease = RequestLease::begin_serial().unwrap();
        let scheduler = lease.client();
        let mut command = Command::new("/bin/sleep");
        command.arg("60");
        let child = scheduler.spawn_child(command).unwrap();
        drop(scheduler);
        let (parent, remote) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            transport::Session::new(remote)
                .respond(|_, _| Err(io::Error::other("terminal fixture failure")))
                .unwrap();
        });
        let mut helper = MacroHelper {
            session: Some(transport::Session::new(HelperTransport::Unix(parent))),
            child: Some(child),
        };
        assert!(helper.load(Path::new("/fixture.so"), "symbol").is_err());
        assert!(helper.session.is_none());
        assert!(helper.load(Path::new("/fixture.so"), "symbol").is_err());
        peer.join().unwrap();
        drop(helper);
        lease.finish().unwrap();
    }
}
