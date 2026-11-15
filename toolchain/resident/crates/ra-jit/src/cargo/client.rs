//! The connection of a `cargo ra-jit` to the user's daemon: finding it, starting it when
//! there is none, and replacing one of another version.

use std::{
    fs::OpenOptions,
    io::{BufRead as _, BufReader, Write as _},
    path::Path,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use anyhow::Context as _;

#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(windows)]
use uds_windows::UnixStream;

use super::{
    daemon::{Checkout, Envelope, Reply, Request, SPARE, home, socket_in, version},
    switch, wrapper,
};
use crate::socket::{connect, private_dir, private_file};

/// How long a daemon that was started is waited for, and one that was told to exit.
const PATIENCE: Duration = Duration::from_secs(10);

const POLL: Duration = Duration::from_millis(5);

/// Whether a daemon is started when none listens.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Start {
    IfMissing,
    Never,
}

/// Sends `request` to the daemon. `None` when there is none and none was to be started;
/// `Reply::Restart` when the one there is is of another version and none was to be started.
pub(crate) fn ask(request: Request, start: Start) -> anyhow::Result<Option<Reply>> {
    let home = home()?;
    let socket = socket_in(&home);
    // A daemon of another version exits for a client that starts its own, and for one that
    // tells it to stop.
    let replaces = start == Start::IfMissing || matches!(request, Request::Stop);
    // A whole build is asked for at once: no cargo builds meanwhile for a spare to load beside.
    let spares = !matches!(&request, Request::Build(checkout) if checkout.config.whole);
    let started = |home: &Path| started(home, spares);
    let request = request.of_this_command();
    let envelope = serde_json::to_string(&Envelope { version: version(), replaces, request })?;
    let mut stream = match (connect(&socket), start) {
        (Ok(stream), _) => stream,
        (Err(_), Start::Never) => return Ok(None),
        (Err(_), Start::IfMissing) => started(&home)?,
    };
    let mut reply = match exchange(&mut stream, &envelope) {
        // A daemon that was on its way out when it was asked is as good as none.
        Err(_) if start == Start::IfMissing => {
            gone(&socket);
            exchange(&mut started(&home)?, &envelope)?
        }
        // One that took its socket with it shut down; one that was killed left it.
        Err(_) if left(&socket) => return Ok(None),
        reply => {
            let reply = reply?;
            if matches!(reply, Reply::Stopping)
                || replaces && matches!(reply, Reply::Restart { .. })
            {
                ended(&mut stream);
            }
            reply
        }
    };
    if let Reply::Restart { .. } = reply {
        if start == Start::Never {
            return Ok(Some(reply));
        }
        gone(&socket);
        reply = exchange(&mut started(&home)?, &envelope)?;
    }
    Ok(Some(reply))
}

/// Cheap non-starting preflight before toolchain discovery or workspace planning.
/// The connection performs a versioned read-only request and cannot replace a daemon.
pub(crate) fn has_ready_worker() -> bool {
    let Ok(home) = home() else { return false };
    let Ok(mut stream) = connect(&socket_in(&home)) else { return false };
    let deadline = Some(Duration::from_millis(250));
    if stream.set_read_timeout(deadline).is_err() || stream.set_write_timeout(deadline).is_err() {
        return false;
    }
    let envelope = Envelope { version: version(), replaces: false, request: Request::Status };
    let Ok(envelope) = serde_json::to_string(&envelope) else { return false };
    matches!(exchange(&mut stream, &envelope), Ok(Reply::Status(status)) if status.workers.iter().any(|worker| {
        worker.checkouts.iter().any(|(_, warmth)| warmth == "ready")
    }))
}

/// Reserve an entire ready-only batch without ever starting or replacing a daemon.
/// `commit` must publish the irreversible execution decision before this connection
/// acknowledges the reservation. Any error after that point is an execution failure.
pub(crate) fn ready(
    checkouts: Vec<Checkout>,
    commit: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<Reply> {
    let home = home()?;
    let stream = match connect(&socket_in(&home)) {
        Ok(stream) => stream,
        Err(_) => {
            return Ok(Reply::ReadyMiss { reason: "no resident daemon is running".to_owned() });
        }
    };
    ready_on(stream, checkouts, commit)
}

fn ready_on(
    mut stream: UnixStream,
    checkouts: Vec<Checkout>,
    commit: impl FnOnce() -> anyhow::Result<()>,
) -> anyhow::Result<Reply> {
    // Admission has a bounded wait. Once committed, the build owns its normal lifetime.
    stream.set_read_timeout(Some(PATIENCE))?;
    stream.set_write_timeout(Some(PATIENCE))?;
    let request = Request::ReadyBuild(checkouts).of_this_command();
    let envelope =
        serde_json::to_string(&Envelope { version: version(), replaces: false, request })?;
    match exchange(&mut stream, &envelope)? {
        Reply::ReadyReserved => {
            commit()?;
            stream.set_read_timeout(None)?;
            stream.write_all(b"commit\n")?;
            read_reply(&mut stream)
        }
        Reply::ReadyMiss { reason } => Ok(Reply::ReadyMiss { reason }),
        Reply::Restart { .. } => {
            Ok(Reply::ReadyMiss { reason: "resident daemon has another build identity".to_owned() })
        }
        other => anyhow::bail!("unexpected ready-only admission reply: {other:?}"),
    }
}

/// Waits for a daemon that answered it ends to have exited, with its workers stopped and its
/// log written: it keeps the connection until its process exits, and no other process has it.
fn ended(stream: &mut UnixStream) {
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let mut rest = [0; 64];
    while matches!(std::io::Read::read(stream, &mut rest), Ok(1..)) {}
}

/// Whether the daemon that did not answer has shut down, rather than been killed.
fn left(socket: &Path) -> bool {
    let asked = Instant::now();
    while socket.exists() && asked.elapsed() < Duration::from_millis(200) {
        std::thread::sleep(POLL);
    }
    !socket.exists()
}

/// Waits for a daemon that is exiting to be gone: it exits once it has answered, and its
/// socket goes with it.
pub(crate) fn gone(socket: &Path) {
    let asked = Instant::now();
    while connect(socket).is_ok() && asked.elapsed() < PATIENCE {
        std::thread::sleep(POLL);
    }
}

/// Starts `daemon` in a process group and a console of its own, and until the result is
/// dropped lets it inherit no handle of this process but those it is given. A process is
/// started with every handle of its parent that can be inherited, and those of a pipe this
/// command writes to are among them, under their own numbers and under those the commands
/// above this one had: whoever reads the pipe would wait for the daemon to end.
#[cfg(windows)]
fn on_its_own(daemon: &mut Command) -> impl Drop + use<> {
    use std::os::windows::process::CommandExt as _;
    use windows_sys::Win32::{
        Foundation::{GetHandleInformation, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation},
        System::Threading::{CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW},
    };

    /// Handles are multiples of four, and a command as short as this one has few.
    const HANDLES: std::ops::RangeInclusive<usize> = 4..=0x1_0000;

    struct Inherited(Vec<HANDLE>);
    impl Drop for Inherited {
        fn drop(&mut self) {
            for &handle in &self.0 {
                // SAFETY: the handle was open and inheritable when it was listed, and
                // nothing closed one of this command's since.
                unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) };
            }
        }
    }

    daemon.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
    let mut inherited = Vec::new();
    for handle in HANDLES.step_by(4).map(|it| it as HANDLE) {
        let mut flags = 0;
        // SAFETY: a number that is no open handle is refused, and `flags` is only written.
        let held = unsafe {
            GetHandleInformation(handle, &mut flags) != 0
                && flags & HANDLE_FLAG_INHERIT != 0
                && SetHandleInformation(handle, HANDLE_FLAG_INHERIT, 0) != 0
        };
        if held {
            inherited.push(handle);
        }
    }
    Inherited(inherited)
}

/// Why no daemon answered: there is none, or one ended without taking its socket with it.
pub(crate) fn why_none() -> String {
    let left = home().map(|it| (socket_in(&it), it.join("daemon.log")));
    match left {
        Ok((socket, log)) if socket.exists() => format!(
            "the daemon is gone without having shut down (killed?), its log is {}",
            log.display()
        ),
        _ => "no daemon is running".to_owned(),
    }
}

fn exchange(stream: &mut UnixStream, envelope: &str) -> anyhow::Result<Reply> {
    stream.write_all(format!("{envelope}\n").as_bytes())?;
    read_reply(stream)
}

fn read_reply(stream: &mut UnixStream) -> anyhow::Result<Reply> {
    let mut line = String::new();
    let read = BufReader::new(stream).read_line(&mut line);
    // A line without its end is one the daemon did not live to finish.
    if !matches!(read, Ok(1..)) || !line.ends_with('\n') {
        let log = home()?.join("daemon.log");
        anyhow::bail!(
            "the daemon ended before it answered (killed?), its log is {}",
            log.display()
        );
    }
    serde_json::from_str(&line).context("the daemon's answer is not one of this version")
}

/// Starts a daemon in a process group of its own, so that what ends this command or its
/// terminal's foreground does not end the daemon, and waits for it to listen.
fn started(home: &Path, spares: bool) -> anyhow::Result<UnixStream> {
    private_dir(home).with_context(|| format!("the daemon's directory {}", home.display()))?;
    let log = private_file(OpenOptions::new().create(true).append(true), &home.join("daemon.log"))?;
    let mut daemon = Command::new(std::env::current_exe()?);
    daemon
        .arg("daemon")
        .env_remove(wrapper::DRIVEN)
        .env_remove(switch::VARIABLE)
        .env_remove(switch::CHAINED);
    if !spares && std::env::var_os(SPARE).is_none() {
        daemon.env(SPARE, "0");
    }
    daemon.stdin(Stdio::null()).stdout(log.try_clone()?).stderr(log);
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut daemon, 0);
    #[cfg(windows)]
    let kept = on_its_own(&mut daemon);
    daemon.spawn().context("starting the daemon")?;
    #[cfg(windows)]
    drop(kept);
    let socket = socket_in(home);
    let asked = Instant::now();
    loop {
        match connect(&socket) {
            Ok(stream) => return Ok(stream),
            Err(error) if asked.elapsed() > PATIENCE => {
                return Err(error).with_context(|| {
                    format!("the daemon did not start, see {}", home.join("daemon.log").display())
                });
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

#[cfg(all(test, unix))]
mod ready_tests {
    use super::*;
    use std::io::{BufRead as _, Write as _};
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering::SeqCst},
    };

    fn receive_request(stream: &UnixStream) {
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        let envelope: Envelope = serde_json::from_str(&line).unwrap();
        assert!(!envelope.replaces);
        assert!(matches!(envelope.request, Request::ReadyBuild(_)));
    }

    fn send(stream: &mut UnixStream, reply: Reply) {
        writeln!(stream, "{}", serde_json::to_string(&reply).unwrap()).unwrap();
    }

    #[test]
    fn explicit_miss_does_not_publish_commitment() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            receive_request(&server);
            send(&mut server, Reply::ReadyMiss { reason: "cold".to_owned() });
        });
        let reply = ready_on(client, Vec::new(), || panic!("miss must not commit")).unwrap();
        assert!(matches!(reply, Reply::ReadyMiss { .. }));
        peer.join().unwrap();
    }

    #[test]
    fn commitment_is_published_before_daemon_acknowledgement() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let committed = Arc::new(AtomicBool::new(false));
        let observed = committed.clone();
        let peer = std::thread::spawn(move || {
            receive_request(&server);
            send(&mut server, Reply::ReadyReserved);
            let mut ack = String::new();
            BufReader::new(&server).read_line(&mut ack).unwrap();
            assert_eq!(ack, "commit\n");
            assert!(observed.load(SeqCst));
            send(
                &mut server,
                Reply::ReadyBuilt {
                    replies: vec![Reply::Cold { reason: "worker lost".to_owned() }],
                },
            );
        });
        let reply = ready_on(client, Vec::new(), || {
            committed.store(true, SeqCst);
            Ok(())
        })
        .unwrap();
        assert!(
            matches!(reply, Reply::ReadyBuilt { .. }),
            "postcommit worker failure stays terminal"
        );
        peer.join().unwrap();
    }

    #[test]
    fn failed_commit_publication_never_acknowledges_reservation() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            receive_request(&server);
            send(&mut server, Reply::ReadyReserved);
            let mut ack = String::new();
            assert_eq!(BufReader::new(&server).read_line(&mut ack).unwrap(), 0);
            assert!(ack.is_empty());
        });
        assert!(ready_on(client, Vec::new(), || anyhow::bail!("receipt channel closed")).is_err());
        peer.join().unwrap();
    }

    #[test]
    fn daemon_lost_after_acknowledgement_never_retries() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let peer = std::thread::spawn(move || {
            receive_request(&server);
            send(&mut server, Reply::ReadyReserved);
            let mut ack = String::new();
            BufReader::new(&server).read_line(&mut ack).unwrap();
            assert_eq!(ack, "commit\n");
        });
        let mut count = 0;
        assert!(
            ready_on(client, Vec::new(), || {
                count += 1;
                Ok(())
            })
            .is_err()
        );
        assert_eq!(count, 1);
        peer.join().unwrap();
    }
}
