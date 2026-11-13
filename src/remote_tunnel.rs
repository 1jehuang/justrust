//! `justrust __tunnel`: an ssh `ProxyCommand` that carries the ssh session to
//! the hosted build machine over a WebSocket on port 443.
//!
//! Hosted ssh listens on 22 (dedicated machines) or 22000+N (pool slots).
//! Hotel, campus and corporate networks routinely drop everything except web
//! ports, which made hosted builds fail with `Connection timed out` and fall
//! back to local. `remote_hosted` selects this transport when the direct port
//! is unreachable.
//!
//! The relay (`GET build/tunnel`) picks the destination itself: the machine
//! leased to the account the bearer key resolves to. Nothing here names a
//! host. ssh still runs end to end with the pinned host key, so the relay
//! only sees ciphertext.
//!
//! stdout carries the ssh byte stream: nothing else may ever print there.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use tungstenite::client::IntoClientRequest;
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Error, Message};

/// Internal argv[1]. Dispatched before anything that could write to stdout.
pub const ARG: &str = "__tunnel";
/// argv[2] for the native Windows worker's relay.
pub const WINDOWS_ARG: &str = "windows";

/// One WebSocket frame of ssh bytes. Well under the relay's message limit.
const CHUNK: usize = 32 * 1024;
/// Inbound frames handled before outbound data gets another turn.
const READ_BURST: usize = 64;

pub fn main() -> ! {
    let windows = std::env::args().nth(2).as_deref() == Some(WINDOWS_ARG);
    match run(windows) {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("justrust tunnel: {e:#}");
            std::process::exit(1)
        }
    }
}

fn run(windows: bool) -> Result<()> {
    let creds = crate::remote_hosted::creds()?;
    crate::remote_status_watch::select_tls_provider();
    let (url, authorization) = creds.tunnel_request(windows);
    let mut request = url.into_client_request()?;
    request.headers_mut().insert("Authorization", authorization);
    let host = request
        .uri()
        .host()
        .ok_or_else(|| anyhow!("relay URL has no host"))?
        .to_owned();
    let port = request
        .uri()
        .port_u16()
        .unwrap_or(if request.uri().scheme_str() == Some("wss") {
            443
        } else {
            80
        });
    let mut tcp = None;
    for address in (host.trim_matches(['[', ']']), port).to_socket_addrs()? {
        if let Ok(stream) = TcpStream::connect_timeout(&address, Duration::from_secs(10)) {
            tcp = Some(stream);
            break;
        }
    }
    let tcp = tcp.with_context(|| format!("cannot reach the relay at {host}:{port}"))?;
    let _ = tcp.set_nodelay(true);
    tcp.set_read_timeout(Some(Duration::from_secs(15)))?;
    tcp.set_write_timeout(Some(Duration::from_secs(15)))?;
    let raw = tcp.try_clone()?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(4 << 20))
        .max_frame_size(Some(4 << 20))
        .max_write_buffer_size(4 << 20);
    let (socket, _) = match tungstenite::client_tls_with_config(request, tcp, Some(config), None) {
        Ok(s) => s,
        Err(tungstenite::HandshakeError::Failure(Error::Http(response))) => {
            bail!("{}", refusal(response.status().as_u16()))
        }
        Err(e) => bail!("relay handshake failed: {e}"),
    };
    pump(socket, raw)
}

/// Why the relay refused, in words a build log reader can act on.
fn refusal(status: u16) -> String {
    match status {
        401 | 403 => "the relay rejected the Jcode API key. Run `justrust login`".into(),
        409 => "no build machine is ready for this account yet".into(),
        s => format!("the relay answered HTTP {s}"),
    }
}

fn would_block(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut | io::ErrorKind::Interrupted
    )
}

/// Wakes the pump when stdin has data, so the socket wait needs no timeout.
#[cfg(unix)]
mod wake {
    use std::io;
    use std::net::TcpStream;
    use std::os::fd::AsRawFd;

    pub struct Waker {
        read: i32,
        write: i32,
    }

    #[derive(Clone, Copy)]
    pub struct Sender(i32);

    impl Sender {
        pub fn wake(&self) {
            // A full pipe already guarantees a wakeup.
            unsafe { libc::write(self.0, [1u8].as_ptr().cast(), 1) };
        }
    }

    impl Waker {
        pub fn new(socket: &TcpStream) -> io::Result<Waker> {
            let mut fds = [0i32; 2];
            if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
                return Err(io::Error::last_os_error());
            }
            for fd in fds {
                unsafe {
                    let flags = libc::fcntl(fd, libc::F_GETFL);
                    libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK);
                }
            }
            socket.set_nonblocking(true)?;
            Ok(Waker {
                read: fds[0],
                write: fds[1],
            })
        }

        pub fn sender(&self) -> Sender {
            Sender(self.write)
        }

        /// Sleep until the socket is readable, stdin produced data, or (when
        /// `output_pending`) the socket accepts more output.
        pub fn wait(&self, socket: &TcpStream, output_pending: bool) {
            let mut fds = [
                libc::pollfd {
                    fd: socket.as_raw_fd(),
                    events: libc::POLLIN | if output_pending { libc::POLLOUT } else { 0 },
                    revents: 0,
                },
                libc::pollfd {
                    fd: self.read,
                    // While output is stuck, more stdin cannot be sent anyway.
                    events: if output_pending { 0 } else { libc::POLLIN },
                    revents: 0,
                },
            ];
            unsafe { libc::poll(fds.as_mut_ptr(), 2, 20_000) };
            let mut sink = [0u8; 256];
            unsafe { libc::read(self.read, sink.as_mut_ptr().cast(), sink.len()) };
        }
    }
}

/// Windows has no poll over a socket and a pipe, so the socket wait is a
/// short WSAPoll and stdin is noticed on the next turn. The socket must be
/// nonblocking like on unix: a blocking write deadlocks against a peer that
/// is itself blocked writing to us.
#[cfg(not(unix))]
mod wake {
    use std::io;
    use std::net::TcpStream;

    pub struct Waker;

    #[derive(Clone, Copy)]
    pub struct Sender;

    impl Sender {
        pub fn wake(&self) {}
    }

    impl Waker {
        pub fn new(socket: &TcpStream) -> io::Result<Waker> {
            socket.set_nonblocking(true)?;
            Ok(Waker)
        }

        pub fn sender(&self) -> Sender {
            Sender
        }

        pub fn wait(&self, socket: &TcpStream, output_pending: bool) {
            use std::os::windows::io::AsRawSocket;
            use windows_sys::Win32::Networking::WinSock::{
                POLLRDNORM, POLLWRNORM, WSAPOLLFD, WSAPoll,
            };
            let mut events = POLLRDNORM as i32;
            if output_pending {
                events |= POLLWRNORM as i32;
            }
            let mut fd = WSAPOLLFD {
                fd: socket.as_raw_socket() as usize,
                events: events as _,
                revents: 0,
            };
            // SAFETY: one valid pollfd for a socket we own.
            let ready = unsafe { WSAPoll(&mut fd, 1, 5) };
            if ready < 0 {
                // Never spin if the poll itself fails.
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
        }
    }
}

type Socket = tungstenite::WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>;

fn pump(mut socket: Socket, raw: TcpStream) -> Result<()> {
    let waker = wake::Waker::new(&raw)?;
    let sender = waker.sender();
    // Bounded: ssh blocks on its pipe when the relay is slower than it.
    let (tx, rx) = mpsc::sync_channel::<Option<Vec<u8>>>(16);
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        loop {
            let mut buf = vec![0u8; CHUNK];
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => {
                    let _ = tx.send(None);
                    sender.wake();
                    return;
                }
                Ok(n) => {
                    buf.truncate(n);
                    if tx.send(Some(buf)).is_err() {
                        return;
                    }
                    sender.wake();
                }
            }
        }
    });
    let mut stdout = io::stdout().lock();
    let mut unsent: Option<Message> = None;
    let mut eof = false;
    loop {
        // ssh -> relay.
        while !eof {
            let message = match unsent.take() {
                Some(m) => m,
                None => match rx.try_recv() {
                    Ok(Some(bytes)) => Message::binary(bytes),
                    Ok(None) | Err(mpsc::TryRecvError::Disconnected) => {
                        eof = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                },
            };
            match socket.write(message) {
                Ok(()) => {}
                Err(Error::WriteBufferFull(m)) => {
                    unsent = Some(*m);
                    break;
                }
                // The frame is queued; only its transmission is pending.
                Err(Error::Io(e)) if would_block(&e) => break,
                Err(Error::ConnectionClosed | Error::AlreadyClosed) => return Ok(()),
                Err(e) => return Err(e).context("relay write"),
            }
        }
        let mut output_pending = unsent.is_some();
        match socket.flush() {
            Ok(()) => {}
            Err(Error::Io(e)) if would_block(&e) => output_pending = true,
            Err(Error::ConnectionClosed | Error::AlreadyClosed) => return Ok(()),
            Err(e) => return Err(e).context("relay write"),
        }
        if eof && !output_pending {
            // ssh closed its side and everything it wrote is on the wire.
            let _ = socket.close(None);
            let _ = socket.flush();
            return Ok(());
        }
        // relay -> ssh.
        let mut more = false;
        for i in 0..READ_BURST {
            match socket.read() {
                Ok(Message::Binary(bytes)) => {
                    stdout.write_all(&bytes).context("ssh closed")?;
                    more = i + 1 == READ_BURST;
                }
                Ok(Message::Close(_)) => {
                    let _ = stdout.flush();
                    return Ok(());
                }
                Ok(_) => {}
                Err(Error::Io(e)) if would_block(&e) => break,
                Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                    let _ = stdout.flush();
                    return Ok(());
                }
                Err(e) => return Err(e).context("relay read"),
            }
        }
        stdout.flush().context("ssh closed")?;
        if !more {
            waker.wait(&raw, output_pending);
        }
    }
}

/// The `ProxyCommand` value that runs this binary as the relay client, or
/// `None` when the executable path cannot be expressed.
pub fn proxy_command(windows_worker: bool) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    proxy_command_for(exe.to_str()?, windows_worker)
}

fn proxy_command_for(exe: &str, windows_worker: bool) -> Option<String> {
    // A running binary replaced by self-update reads back with this suffix on
    // Linux. The path itself is the new release.
    let exe = exe.strip_suffix(" (deleted)").unwrap_or(exe);
    if exe.is_empty() || exe.contains(['\n', '\r', '\0']) {
        return None;
    }
    let quoted = if cfg!(windows) {
        if exe.contains('"') {
            return None;
        }
        format!("\"{exe}\"")
    } else {
        crate::remote_build::shell_quote(exe)
    };
    // ssh expands %-tokens in ProxyCommand.
    let mut command = format!("{} {ARG}", quoted.replace('%', "%%"));
    if windows_worker {
        command.push(' ');
        command.push_str(WINDOWS_ARG);
    }
    Some(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proxy_command_quotes_paths_and_escapes_ssh_tokens() {
        assert_eq!(
            proxy_command_for("/usr/local/bin/justrust", false).as_deref(),
            Some("/usr/local/bin/justrust __tunnel")
        );
        assert_eq!(
            proxy_command_for("/usr/local/bin/justrust", true).as_deref(),
            Some("/usr/local/bin/justrust __tunnel windows")
        );
        // A replaced binary still names the installed path.
        assert_eq!(
            proxy_command_for("/home/u/.cargo/bin/justrust (deleted)", false).as_deref(),
            Some("/home/u/.cargo/bin/justrust __tunnel")
        );
        if cfg!(unix) {
            // Spaces and quotes cannot split the command or start another.
            assert_eq!(
                proxy_command_for("/home/a b/it's/justrust", false).as_deref(),
                Some("'/home/a b/it'\\''s/justrust' __tunnel")
            );
            assert_eq!(
                proxy_command_for("/opt/x; rm -rf ~/justrust", false).as_deref(),
                Some("'/opt/x; rm -rf ~/justrust' __tunnel")
            );
            // %h and friends are ssh tokens: they must stay literal.
            assert_eq!(
                proxy_command_for("/opt/100%h/justrust", false).as_deref(),
                Some("/opt/100%%h/justrust __tunnel")
            );
        }
        assert_eq!(proxy_command_for("", false), None);
        assert_eq!(proxy_command_for("/a\nb", false), None);
    }

    #[test]
    fn refusals_say_what_to_do() {
        assert!(refusal(401).contains("justrust login"));
        assert!(refusal(409).contains("no build machine"));
        assert!(refusal(502).contains("502"));
    }

    /// The whole client path against a local plain-WebSocket echo relay:
    /// bytes arrive intact and in order in both directions, including a
    /// payload far larger than one frame and than the socket buffers.
    #[test]
    fn pump_relays_large_payloads_both_ways() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let mut total = 0usize;
            let mut sum = 0u64;
            loop {
                match ws.read() {
                    Ok(Message::Binary(b)) => {
                        total += b.len();
                        sum += b.iter().map(|&x| x as u64).sum::<u64>();
                        ws.send(Message::binary(b.to_vec())).unwrap();
                    }
                    Ok(Message::Close(_)) | Err(_) => return (total, sum),
                    Ok(_) => {}
                }
            }
        });
        let tcp = TcpStream::connect(address).unwrap();
        let raw = tcp.try_clone().unwrap();
        let (mut socket, _) = tungstenite::client(
            format!("ws://{address}/v1/build/tunnel"),
            tungstenite::stream::MaybeTlsStream::Plain(tcp),
        )
        .unwrap();
        // Drive the same nonblocking write/flush/read discipline as `pump`,
        // with an in-memory source and sink instead of stdin and stdout.
        let waker = wake::Waker::new(&raw).unwrap();
        let payload: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let mut chunks = payload.chunks(CHUNK);
        let mut unsent: Option<Message> = None;
        let mut echoed = Vec::with_capacity(payload.len());
        let mut done_sending = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while echoed.len() < payload.len() {
            assert!(std::time::Instant::now() < deadline, "relay stalled");
            while !done_sending {
                let message = match unsent.take() {
                    Some(m) => m,
                    None => match chunks.next() {
                        Some(c) => Message::binary(c.to_vec()),
                        None => {
                            done_sending = true;
                            break;
                        }
                    },
                };
                match socket.write(message) {
                    Ok(()) => {}
                    Err(Error::WriteBufferFull(m)) => {
                        unsent = Some(*m);
                        break;
                    }
                    Err(Error::Io(e)) if would_block(&e) => break,
                    Err(e) => panic!("{e}"),
                }
            }
            let mut output_pending = unsent.is_some() || !done_sending;
            match socket.flush() {
                Ok(()) => {}
                Err(Error::Io(e)) if would_block(&e) => output_pending = true,
                Err(e) => panic!("{e}"),
            }
            loop {
                match socket.read() {
                    Ok(Message::Binary(b)) => echoed.extend_from_slice(&b),
                    Ok(_) => {}
                    Err(Error::Io(e)) if would_block(&e) => break,
                    Err(e) => panic!("{e}"),
                }
            }
            if echoed.len() < payload.len() {
                waker.wait(&raw, output_pending);
            }
        }
        assert!(echoed == payload, "echo differs from what was sent");
        raw.set_nonblocking(false).unwrap();
        let _ = socket.close(None);
        let _ = socket.flush();
        drop(socket);
        let (total, sum) = server.join().unwrap();
        assert_eq!(total, payload.len());
        assert_eq!(sum, payload.iter().map(|&x| x as u64).sum::<u64>());
    }
}
