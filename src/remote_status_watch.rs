//! Remote-status wakeups: narrow filesystem events plus account-scoped hosted
//! WebSocket hints. No periodic hosted status requests while subscribed.

#[cfg(target_os = "linux")]
use std::collections::HashMap;
use std::io;
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
#[cfg(target_os = "linux")]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Error, Message};

pub const LOCAL: u32 = 1;
pub const HOSTED: u32 = 2;
pub const ONLINE: u32 = 4;
pub const OFFLINE: u32 = 8;
pub const AUTH: u32 = 16;
pub const CLOSED: u32 = 32;

#[derive(Default)]
struct Signals {
    bits: AtomicU32,
    lock: Mutex<()>,
    cv: Condvar,
    #[cfg(target_os = "linux")]
    fd: Option<std::os::fd::OwnedFd>,
}

impl Signals {
    fn new() -> Self {
        #[cfg(target_os = "linux")]
        {
            use std::os::fd::FromRawFd;
            // SAFETY: syscall returning a newly owned descriptor.
            let raw = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
            Self {
                fd: (raw >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) }),
                ..Default::default()
            }
        }
        #[cfg(not(target_os = "linux"))]
        Self::default()
    }

    fn send(&self, bits: u32) {
        // Serialize with the non-Linux condition-variable wait to avoid a
        // notification between its predicate check and entering the wait.
        let _guard = self.lock.lock().unwrap();
        self.bits.fetch_or(bits, Ordering::SeqCst);
        #[cfg(target_os = "linux")]
        if let Some(fd) = &self.fd {
            use std::os::fd::AsRawFd;
            let one = 1u64;
            // SAFETY: nonblocking eventfd, valid 8-byte value. EAGAIN means
            // a wakeup is already pending, and the bits still retain it.
            unsafe { libc::write(fd.as_raw_fd(), (&one as *const u64).cast(), 8) };
        }
        self.cv.notify_all();
    }
}

#[derive(Default)]
struct Connection {
    generation: AtomicU64,
    stop: AtomicBool,
    connected: AtomicBool,
    socket: Mutex<Option<TcpStream>>,
    lock: Mutex<()>,
    retry: Condvar,
}

impl Connection {
    fn interrupt(&self) {
        let _guard = self.lock.lock().unwrap();
        self.generation.fetch_add(1, Ordering::SeqCst);
        self.connected.store(false, Ordering::SeqCst);
        if let Some(s) = self.socket.lock().unwrap().take() {
            let _ = s.shutdown(Shutdown::Both);
        }
        self.retry.notify_all();
    }

    fn pause(&self, generation: u64, duration: Option<Duration>) {
        let guard = self.lock.lock().unwrap();
        let unchanged = |_: &mut ()| {
            !self.stop.load(Ordering::SeqCst)
                && self.generation.load(Ordering::SeqCst) == generation
        };
        if let Some(d) = duration {
            drop(self.retry.wait_timeout_while(guard, d, unchanged).unwrap());
        } else {
            drop(self.retry.wait_while(guard, unchanged).unwrap());
        }
    }
}

/// A socket read timeout is a transport keepalive, not a status refresh.
/// The server auto-responds to the tiny text ping without waking its DO.
const KEEPALIVE: Duration = Duration::from_secs(60);

fn status_hint(text: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(text)
        .is_ok_and(|v| v["type"] == "status_changed" || v["type"] == "hello" || v["hello"] == true)
}

fn select_tls_provider() {
    // tungstenite deliberately disables rustls's default features. Select an
    // actual provider explicitly so WSS cannot panic while plain-WS tests pass.
    // A provider already selected by the embedding process remains in place.
    let _ = rustls::crypto::ring::default_provider().install_default();
}

fn hosted_connection(
    control: &Connection,
    signals: &Signals,
    generation: u64,
    creds: &crate::remote_hosted::Creds,
) -> anyhow::Result<()> {
    select_tls_provider();
    let (url, authorization) = creds.status_event_request();
    let mut request = url.into_client_request()?;
    request.headers_mut().insert("Authorization", authorization);
    let host = request
        .uri()
        .host()
        .ok_or_else(|| io::Error::other("missing host"))?;
    let port = request
        .uri()
        .port_u16()
        .unwrap_or(if request.uri().scheme_str() == Some("wss") {
            443
        } else {
            80
        });
    let addresses = (host.trim_matches(['[', ']']), port).to_socket_addrs()?;
    let mut tcp = None;
    for address in addresses {
        if control.stop.load(Ordering::SeqCst)
            || control.generation.load(Ordering::SeqCst) != generation
        {
            return Ok(());
        }
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(10)) {
            tcp = Some(stream);
            break;
        }
    }
    let tcp = tcp.ok_or_else(|| io::Error::other("event connection unavailable"))?;
    tcp.set_read_timeout(Some(Duration::from_secs(10)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(10)))?;
    let read_control = tcp.try_clone()?;
    {
        let _guard = control.lock.lock().unwrap();
        if control.stop.load(Ordering::SeqCst)
            || control.generation.load(Ordering::SeqCst) != generation
        {
            return Ok(());
        }
        *control.socket.lock().unwrap() = Some(tcp.try_clone()?);
    }
    let config = WebSocketConfig::default()
        .max_message_size(Some(16 * 1024))
        .max_frame_size(Some(16 * 1024));
    let (mut socket, _) = tungstenite::client_tls_with_config(request, tcp, Some(config), None)?;
    read_control.set_read_timeout(Some(KEEPALIVE))?;
    {
        let _guard = control.lock.lock().unwrap();
        if control.stop.load(Ordering::SeqCst)
            || control.generation.load(Ordering::SeqCst) != generation
        {
            return Ok(());
        }
        control.connected.store(true, Ordering::SeqCst);
    }
    signals.send(ONLINE | HOSTED); // reconnect always takes a fresh snapshot
    let mut missed = false;
    loop {
        if control.stop.load(Ordering::SeqCst)
            || control.generation.load(Ordering::SeqCst) != generation
        {
            return Ok(());
        }
        match socket.read() {
            Ok(Message::Text(text)) => {
                missed = false;
                if status_hint(text.as_str()) {
                    signals.send(HOSTED);
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(Message::Ping(_)) => {
                socket.flush()?;
            }
            Ok(_) => {
                missed = false;
            }
            Err(Error::Io(e))
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) =>
            {
                if missed {
                    break;
                }
                socket.send(Message::Text("ping".into()))?;
                missed = true;
            }
            Err(_) => break,
        }
    }
    Ok(())
}

fn hosted_loop(control: Arc<Connection>, signals: Arc<Signals>) {
    let mut backoff = 1;
    while !control.stop.load(Ordering::SeqCst) {
        let generation = control.generation.load(Ordering::SeqCst);
        let hosted = matches!(
            crate::remote_backend::load(),
            Some(crate::remote_backend::Backend::Hosted)
        );
        let creds = hosted
            .then(crate::remote_hosted::creds)
            .and_then(Result::ok);
        let Some(creds) = creds else {
            // Signing in/backend changes wake this explicitly via filesystem.
            control.pause(generation, None);
            continue;
        };
        let started = Instant::now();
        // Never log the request, headers or handshake error: they may carry
        // credentials. The ordinary status snapshot supplies useful errors.
        let _ = hosted_connection(&control, &signals, generation, &creds);
        control.connected.store(false, Ordering::SeqCst);
        control.socket.lock().unwrap().take();
        signals.send(OFFLINE);
        if started.elapsed() >= KEEPALIVE || control.generation.load(Ordering::SeqCst) != generation
        {
            backoff = 1;
        }
        control.pause(generation, Some(Duration::from_secs(backoff)));
        backoff = (backoff * 2).min(60);
    }
}

pub struct RemoteStatusWatch {
    signals: Arc<Signals>,
    control: Arc<Connection>,
    #[cfg(target_os = "linux")]
    files: FileEvents,
}

impl RemoteStatusWatch {
    pub fn new() -> Self {
        // Status already creates this directory while collecting local state.
        // Establish it before registering, not between snapshot and watch.
        let _ = crate::remote::dir();
        let signals = Arc::new(Signals::new());
        let control = Arc::new(Connection::default());
        #[cfg(target_os = "linux")]
        let files = FileEvents::new(); // subscribe before collecting state
        let c = control.clone();
        let s = signals.clone();
        std::thread::spawn(move || hosted_loop(c, s));
        Self {
            signals,
            control,
            #[cfg(target_os = "linux")]
            files,
        }
    }

    pub fn restart_hosted(&self) {
        self.control.interrupt();
    }

    pub fn online(&self) -> bool {
        self.control.connected.load(Ordering::SeqCst)
    }

    /// Returns coalesced event bits, zero for the explicit measurement or
    /// compatibility-fallback deadline. Linux watches stdout for orphan exit.
    pub fn wait(&mut self, timeout: Option<Duration>) -> u32 {
        #[cfg(target_os = "linux")]
        if let Some(signal_fd) = &self.signals.fd {
            use std::os::fd::AsRawFd;
            self.files.refresh();
            let end = timeout.map(|t| Instant::now() + t);
            loop {
                let bits = self.signals.bits.swap(0, Ordering::SeqCst);
                if bits != 0 {
                    return bits;
                }
                let mut pfds = [
                    libc::pollfd {
                        fd: signal_fd.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: self.files.fd.as_ref().map_or(-1, AsRawFd::as_raw_fd),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: libc::STDOUT_FILENO,
                        events: 0,
                        revents: 0,
                    },
                ];
                let timeout = end.map(|e| e.saturating_duration_since(Instant::now()));
                let ms = timeout.map_or(-1, |d| d.as_millis().min(i32::MAX as u128) as i32);
                // SAFETY: three valid pollfd slots; -1 intentionally ignored.
                let n = unsafe { libc::poll(pfds.as_mut_ptr(), pfds.len() as _, ms) };
                if n < 0 {
                    if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    return OFFLINE;
                }
                if pfds[2].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                    return CLOSED;
                }
                if n == 0 {
                    return 0;
                }
                if pfds[0].revents & libc::POLLIN != 0 {
                    let mut count = 0u64;
                    unsafe { libc::read(pfds[0].fd, (&mut count as *mut u64).cast(), 8) };
                }
                let bits = self.signals.bits.swap(0, Ordering::SeqCst)
                    | if pfds[1].revents & libc::POLLIN != 0 {
                        self.files.drain()
                    } else {
                        0
                    };
                if bits != 0 {
                    return bits;
                }
                // Unrelated files and our own cache writes never recollect.
            }
        }
        let mut guard = self.signals.lock.lock().unwrap();
        if self.signals.bits.load(Ordering::SeqCst) == 0 {
            let duration = timeout.unwrap_or(Duration::from_secs(60));
            guard = self.signals.cv.wait_timeout(guard, duration).unwrap().0;
        }
        drop(guard);
        self.signals.bits.swap(0, Ordering::SeqCst)
    }

    pub fn has_files(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            self.signals.fd.is_some() && self.files.fd.is_some() && !self.files.paths.is_empty()
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }
}

impl Drop for RemoteStatusWatch {
    fn drop(&mut self) {
        self.control.stop.store(true, Ordering::SeqCst);
        self.control.interrupt();
    }
}

#[cfg(target_os = "linux")]
struct FileEvents {
    fd: Option<std::os::fd::OwnedFd>,
    paths: HashMap<i32, PathBuf>,
}

#[cfg(target_os = "linux")]
impl FileEvents {
    fn new() -> Self {
        use std::os::fd::FromRawFd;
        let raw = unsafe { libc::inotify_init1(libc::IN_CLOEXEC | libc::IN_NONBLOCK) };
        let mut s = Self {
            fd: (raw >= 0).then(|| unsafe { std::os::fd::OwnedFd::from_raw_fd(raw) }),
            paths: HashMap::new(),
        };
        s.refresh();
        s
    }

    fn roots() -> Vec<PathBuf> {
        let mut roots = Vec::new();
        if let Ok(home) = crate::paths::home() {
            roots.extend([
                home.clone(),
                home.join("remote"),
                home.join("remote/hosted"),
            ]);
        }
        if let Some(file) = crate::remote_hosted::env_file_path()
            && let Some(parent) = file.parent()
        {
            roots.push(parent.to_path_buf());
        }
        // Also observe parent creation/replacement. In particular a first
        // login may create config/jcode and its credential in one operation.
        let mut parents = Vec::new();
        for root in &roots {
            let mut parent = root.parent();
            while let Some(path) = parent {
                parents.push(path.to_path_buf());
                if path.is_dir() {
                    break;
                }
                parent = path.parent();
            }
        }
        roots.extend(parents);
        roots.sort();
        roots.dedup();
        roots
    }

    fn refresh(&mut self) {
        use std::os::fd::AsRawFd;
        let Some(fd) = &self.fd else { return };
        for path in Self::roots() {
            if self.paths.values().any(|p| *p == path) {
                continue;
            }
            // Do not create account directories for a status display. Watch
            // existing parents and register children after they appear.
            let Ok(c) = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()) else {
                continue;
            };
            let mask = libc::IN_CREATE
                | libc::IN_CLOSE_WRITE
                | libc::IN_DELETE
                | libc::IN_MOVED_TO
                | libc::IN_MOVED_FROM
                | libc::IN_DELETE_SELF
                | libc::IN_MOVE_SELF;
            let wd = unsafe { libc::inotify_add_watch(fd.as_raw_fd(), c.as_ptr(), mask) };
            if wd >= 0 {
                self.paths.insert(wd, path);
            }
        }
    }

    fn name_bits(name: &[u8]) -> u32 {
        match name {
            b"backend.json"
            | b"jcode-subscription.env"
            | b"subscription.json"
            | b"credentials.json"
            | b"remote"
            | b"hosted"
            | b"jcode"
            | b"config"
            | b".config"
            | b".justrust" => LOCAL | AUTH,
            b"daemon-status" | b"daemon.sock" | b"state.json" | b"pending-login.json" => LOCAL,
            n if n.starts_with(b"cm-") => LOCAL,
            _ => 0,
        }
    }

    fn drain(&mut self) -> u32 {
        use std::os::fd::AsRawFd;
        let Some(fd) = &self.fd else { return 0 };
        let raw = fd.as_raw_fd();
        let mut bits = 0;
        let mut lost = false;
        let mut buf = [0u8; 16384];
        loop {
            let n = unsafe { libc::read(raw, buf.as_mut_ptr().cast(), buf.len()) };
            if n <= 0 {
                break;
            }
            let mut offset = 0;
            while offset + std::mem::size_of::<libc::inotify_event>() <= n as usize {
                let ev = unsafe {
                    std::ptr::read_unaligned(buf.as_ptr().add(offset).cast::<libc::inotify_event>())
                };
                let start = offset + std::mem::size_of::<libc::inotify_event>();
                let end = start + ev.len as usize;
                if end > n as usize {
                    break;
                }
                if ev.mask
                    & (libc::IN_Q_OVERFLOW
                        | libc::IN_DELETE_SELF
                        | libc::IN_MOVE_SELF
                        | libc::IN_IGNORED)
                    != 0
                {
                    bits |= LOCAL | AUTH;
                    lost = true;
                }
                let name = &buf[start..end];
                let name = &name[..name.iter().position(|c| *c == 0).unwrap_or(name.len())];
                // Ignore CREATE for files written in place until CLOSE_WRITE.
                // Directory creation registers new watches before the rescan.
                if ev.mask & libc::IN_ISDIR != 0
                    || ev.mask & libc::IN_CREATE == 0
                    || name == b"daemon.sock"
                    || name.starts_with(b"cm-")
                {
                    bits |= Self::name_bits(name);
                }
                offset = end;
            }
        }
        if lost {
            // Re-subscribe by pathname, including descendants of a replaced
            // directory, rather than following renamed stale inodes.
            *self = Self::new();
        } else {
            self.refresh();
        }
        bits
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tls_provider_is_configured_for_secure_websockets() {
        select_tls_provider();
        let _config = rustls::ClientConfig::builder()
            .with_root_certificates(rustls::RootCertStore::empty())
            .with_no_client_auth();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }

    #[test]
    fn only_known_hints_refresh_hosted_state() {
        assert!(status_hint(r#"{"type":"status_changed"}"#));
        assert!(status_hint(r#"{"type":"hello"}"#));
        assert!(status_hint(r#"{"hello":true}"#));
        assert!(!status_hint("pong"));
        assert!(!status_hint(r#"{"type":"ops"}"#));
        assert!(!status_hint("broken"));
    }

    #[test]
    fn signals_coalesce_without_losing_types() {
        let s = Signals::new();
        s.send(HOSTED);
        s.send(LOCAL);
        assert_eq!(s.bits.swap(0, Ordering::SeqCst), HOSTED | LOCAL);
        assert_eq!(s.bits.load(Ordering::SeqCst), 0);
    }

    #[test]
    // tungstenite's Callback API fixes this error type to an HTTP response.
    #[allow(clippy::result_large_err)]
    fn websocket_authenticates_and_receives_change_hints() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (tcp, _) = listener.accept().unwrap();
            tcp.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut socket = tungstenite::accept_hdr(
                tcp,
                |request: &tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.uri().path(), "/v1/build/events");
                    assert!(request.uri().query().is_none());
                    assert_eq!(request.headers()["Authorization"], "Bearer test-event-key");
                    Ok(response)
                },
            )
            .unwrap();
            socket
                .send(Message::Text(r#"{"type":"hello"}"#.into()))
                .unwrap();
            socket
                .send(Message::Text(r#"{"type":"status_changed"}"#.into()))
                .unwrap();
            socket.close(None).unwrap();
        });
        let creds = crate::remote_hosted::Creds::new(
            format!("http://{address}/v1"),
            "test-event-key".into(),
        )
        .unwrap();
        let signals = Signals::new();
        let control = Connection::default();
        hosted_connection(&control, &signals, 0, &creds).unwrap();
        assert_eq!(
            signals.bits.load(Ordering::SeqCst) & (HOSTED | ONLINE),
            HOSTED | ONLINE
        );
        server.join().unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn real_inotify_filters_writes_and_detects_socket_creation() {
        use std::os::fd::AsRawFd;
        use std::os::unix::net::UnixListener;
        let dir = std::env::temp_dir().join(format!(
            "justrust-status-watch-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let mut files = FileEvents::new();
        let c = std::ffi::CString::new(dir.as_os_str().as_encoded_bytes()).unwrap();
        let mask = libc::IN_CREATE | libc::IN_CLOSE_WRITE | libc::IN_DELETE | libc::IN_MOVED_TO;
        let wd = unsafe {
            libc::inotify_add_watch(files.fd.as_ref().unwrap().as_raw_fd(), c.as_ptr(), mask)
        };
        assert!(wd >= 0);
        files.paths.insert(wd, dir.clone());
        let _ = files.drain();
        std::fs::write(dir.join("status.json"), b"cached").unwrap();
        assert_eq!(files.drain(), 0);
        std::fs::write(dir.join("daemon-status"), b"1\n").unwrap();
        assert_eq!(files.drain() & LOCAL, LOCAL);
        let listener = UnixListener::bind(dir.join("daemon.sock")).unwrap();
        assert_eq!(files.drain() & LOCAL, LOCAL);
        drop(listener);
        std::fs::remove_file(dir.join("daemon.sock")).unwrap();
        assert_eq!(files.drain() & LOCAL, LOCAL);
        std::fs::write(dir.join("jcode-subscription.env"), b"changed").unwrap();
        assert_eq!(files.drain() & AUTH, AUTH);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn cache_writes_and_logs_do_not_feedback_into_status() {
        for name in [
            b"status.json".as_slice(),
            b"probe-hosted.json",
            b"daemon.log",
            b"roots.json",
            b"events.log",
        ] {
            assert_eq!(FileEvents::name_bits(name), 0);
        }
        assert_eq!(FileEvents::name_bits(b"daemon-status"), LOCAL);
        assert_eq!(
            FileEvents::name_bits(b"jcode-subscription.env"),
            LOCAL | AUTH
        );
        assert_eq!(FileEvents::name_bits(b"cm-host"), LOCAL);
    }
}
