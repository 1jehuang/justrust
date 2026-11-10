//! Durable hosted SSH transport. Reattachment never starts or resubmits a build.
//!
//! A private broker owns the original agent and its pipes. SSH connections only
//! carry sequenced input and journal offsets. Losing SSH therefore does not close
//! the agent's stdin. A missing broker (including host reboot) is a lost job, not
//! permission to create another execution.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::remote_backend::Target;
use crate::remote_build::PinnedSessionExecutable;
use crate::remote_proto::{Msg, read_frame, write_frame};
use crate::sys::Detach;

pub const ARG: &str = "__remote-session";
pub const SOURCE_ID: &str = "JUSTRUST_SESSION_SOURCE_ID";
const GRACE: Duration = Duration::from_secs(30);
const RECONNECTS: usize = 3;
const MAX_JOURNAL: u64 = 256 << 20;
const MAX_PACKET: usize = 80 << 20;
const MAX_PENDING: usize = 128 << 20;
const AGENT_WRITE_TIMEOUT: Duration = Duration::from_secs(5);

fn nonblocking(input: &std::process::ChildStdin) -> std::io::Result<()> {
    let fd = input.as_raw_fd();
    // SAFETY: fd is the live, broker-owned pipe descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Only EAGAIN/EINTR are retried. The descriptor must be nonblocking so a
/// wedged agent cannot trap cancellation behind an unbounded write syscall.
fn write_agent(
    input: &mut std::process::ChildStdin,
    mut bytes: &[u8],
    deadline: Instant,
) -> std::io::Result<()> {
    while !bytes.is_empty() {
        if Instant::now() >= deadline {
            return Err(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "original agent stopped reading its input",
            ));
        }
        match input.write(bytes) {
            Ok(0) => return Err(std::io::ErrorKind::WriteZero.into()),
            Ok(n) => bytes = &bytes[n..],
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                let left = deadline.saturating_duration_since(Instant::now());
                let timeout = left.as_millis().min(100) as i32;
                let mut poll = libc::pollfd {
                    fd: input.as_raw_fd(),
                    events: libc::POLLOUT,
                    revents: 0,
                };
                // SAFETY: one initialized pollfd, live descriptor, bounded timeout.
                if unsafe { libc::poll(&mut poll, 1, timeout) } < 0 {
                    let error = std::io::Error::last_os_error();
                    if error.kind() != std::io::ErrorKind::Interrupted {
                        return Err(error);
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Identity {
    version: u32,
    session: String,
    incarnation: String,
    machine: String,
    boot: String,
    binary: String,
    prefix: String,
    executable: PinnedSessionExecutable,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "t")]
enum Packet {
    Attach {
        session: String,
        prefix: String,
        identity: Option<Identity>,
        offset: u64,
    },
    Ready {
        identity: Identity,
        accepted: u64,
    },
    Input {
        seq: u64,
    },
    Ack {
        seq: u64,
    },
    Output {
        offset: u64,
    },
    Close,
    Pulse,
    End,
    Fatal {
        message: String,
    },
}

fn send(w: &mut impl Write, packet: &Packet, data: &[u8]) -> Result<()> {
    let header = serde_json::to_vec(packet)?;
    w.write_all(&(header.len() as u32).to_be_bytes())?;
    w.write_all(&header)?;
    w.write_all(&(data.len() as u32).to_be_bytes())?;
    w.write_all(data)?;
    w.flush()?;
    Ok(())
}

fn receive(r: &mut impl Read) -> Result<(Packet, Vec<u8>)> {
    fn part(r: &mut impl Read, cap: usize) -> Result<Vec<u8>> {
        let mut n = [0; 4];
        r.read_exact(&mut n)?;
        let n = u32::from_be_bytes(n) as usize;
        if n > cap {
            bail!("invalid durable-session packet length");
        }
        let mut bytes = vec![0; n];
        r.read_exact(&mut bytes)?;
        Ok(bytes)
    }
    let header = part(r, 16 << 10)?;
    let data = part(r, MAX_PACKET)?;
    Ok((serde_json::from_slice(&header)?, data))
}

// Binary agent frames need not contain newlines. Stdout is line buffered even
// when piped, so every live chunk must be flushed before awaiting more input.
fn publish(output: &mut impl Write, bytes: &[u8]) -> std::io::Result<()> {
    output.write_all(bytes)?;
    output.flush()
}

fn copy_live(input: &mut impl Read, output: &mut impl Write) -> std::io::Result<()> {
    let mut bytes = [0; 64 << 10];
    loop {
        let count = match input.read(&mut bytes) {
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            result => result?,
        };
        if count == 0 {
            return Ok(());
        }
        publish(output, &bytes[..count])?;
    }
}

fn encode(msg: &Msg, data: &[u8]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, msg, data)?;
    Ok(bytes)
}

fn token() -> Result<String> {
    let mut bytes = [0; 32];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn session_dir(session: &str) -> Result<PathBuf> {
    if session.len() != 64 || !session.bytes().all(|c| c.is_ascii_hexdigit()) {
        bail!("invalid durable-session capability");
    }
    // Keep sockaddr_un below its 108-byte limit on ordinary home paths.
    // The full 256-bit identity is still checked in the attachment handshake.
    Ok(crate::paths::home()?
        .join("remote/sessions")
        .join(&session[..32]))
}

fn save(path: &Path, value: &impl Serialize) -> Result<()> {
    let tmp = path.with_extension("pending");
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec(value)?)?;
    file.sync_data()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

#[derive(Serialize)]
struct Job {
    session: Identity,
    invocation: Msg,
    source: String,
    state: String,
    exit: Option<i32>,
}

struct BrokerState {
    accepted: u64,
    length: u64,
    jobs: BTreeMap<u64, Job>,
    fatal: Option<String>,
    done: bool,
    inputs: BTreeMap<u64, String>,
}

struct Broker {
    identity: Identity,
    dir: PathBuf,
    input: Mutex<Option<std::process::ChildStdin>>,
    state: Mutex<BrokerState>,
    changed: Condvar,
}

impl Broker {
    fn fail(&self, error: impl std::fmt::Display) {
        let mut state = self.state.lock().unwrap();
        if state.fatal.is_none() {
            state.fatal = Some(error.to_string());
        }
        self.changed.notify_all();
    }

    fn input(&self, seq: u64, bytes: &[u8]) -> Result<u64> {
        let mut hash = crate::depcache::H128::new();
        hash.field(bytes);
        let fingerprint = hash.hex();
        let mut state = self.state.lock().unwrap();
        if seq <= state.accepted {
            if state.inputs.get(&seq) != Some(&fingerprint) {
                bail!("previously accepted session input changed (lostjob)");
            }
            return Ok(state.accepted);
        }
        if state.inputs.len() >= 1_000_000 {
            bail!("durable-session input receipt limit exhausted");
        }
        if seq != state.accepted + 1 || state.done || state.fatal.is_some() {
            bail!("durable-session input sequence/state mismatch");
        }
        let mut frame = bytes;
        let (mut msg, data) = read_frame(&mut frame)?.context("empty session input")?;
        if !frame.is_empty() {
            bail!("multiple agent frames in one session input");
        }
        let job = match &mut msg {
            Msg::Run { id, env, .. } | Msg::RunSync { id, env, .. } => {
                let source = env
                    .iter()
                    .position(|(k, _)| k == SOURCE_ID)
                    .map(|i| env.remove(i).1)
                    .context("run has no acknowledged source identity")?;
                if source.is_empty() || state.jobs.contains_key(id) {
                    bail!("duplicate or unbound durable-session invocation");
                }
                Some((*id, source))
            }
            _ => None,
        };
        if let Some((id, source)) = job {
            let job = Job {
                session: self.identity.clone(),
                invocation: msg.clone(),
                source,
                state: "submission_pending".into(),
                exit: None,
            };
            // Receipt exists before execution can be accepted. A crash at any
            // later point is ambiguous and must never turn into resubmission.
            save(&self.dir.join(format!("job-{id}.json")), &job)?;
            state.jobs.insert(id, job);
        }
        let forwarded = encode(&msg, &data)?;
        drop(state);
        // Drain agent output even while its input pipe is full.
        let mut input = self.input.lock().unwrap();
        if let Err(error) = write_agent(
            input.as_mut().context("original agent is gone (lostjob)")?,
            &forwarded,
            Instant::now() + AGENT_WRITE_TIMEOUT,
        ) {
            // A partial frame is ambiguous. Close its pipe, never append a
            // control frame into the unfinished payload or replay the input.
            input.take();
            return Err(error.into());
        }
        let mut state = self.state.lock().unwrap();
        state.accepted = seq;
        state.inputs.insert(seq, fingerprint);
        self.changed.notify_all();
        // Never acknowledge before the whole frame reached the original agent.
        // This process alone owns sequence deduplication. A broker crash is
        // unrecoverable, even if a receipt remains on disk.
        Ok(seq)
    }

    fn output(&self, msg: &Msg, bytes: &[u8], journal: &mut File) -> Result<()> {
        let frame = encode(msg, bytes)?;
        let mut state = self.state.lock().unwrap();
        if state.length + frame.len() as u64 > MAX_JOURNAL {
            bail!("durable-session diagnostic journal exhausted its 256 MiB limit");
        }
        let terminal = match msg {
            Msg::Exit { id, code } => Some((*id, "finished", Some(*code))),
            Msg::Error { id, .. } if *id != 0 => Some((*id, "failed", None)),
            _ => None,
        };
        if let Some((id, status, exit)) = terminal
            && let Some(job) = state.jobs.get_mut(&id)
        {
            job.state = status.into();
            job.exit = exit;
            save(&self.dir.join(format!("job-{id}.json")), job)?;
        }
        journal.write_all(&frame)?;
        journal.flush()?;
        state.length += frame.len() as u64;
        self.changed.notify_all();
        Ok(())
    }

    fn cancel(&self, deadline: Instant) {
        let ids: Vec<_> = loop {
            if let Ok(state) = self.state.try_lock() {
                break state
                    .jobs
                    .iter()
                    .filter(|(_, job)| job.state == "submission_pending")
                    .map(|(id, _)| *id)
                    .collect();
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        if ids.is_empty() {
            return;
        }
        loop {
            if let Ok(mut input) = self.input.try_lock() {
                if let Some(input) = input.as_mut() {
                    for id in ids {
                        if let Ok(frame) = encode(&Msg::Cancel { id }, &[]) {
                            // Normal graceful cancellation is still attempted.
                            // A full pipe cannot extend the shared deadline.
                            if write_agent(input, &frame, deadline).is_err() {
                                break;
                            }
                        }
                    }
                }
                return;
            }
            if Instant::now() >= deadline {
                return;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn check_prefix(prefix: &str) -> Result<()> {
    anyhow::ensure!(
        !prefix.contains('\0') && (prefix.is_empty() || Path::new(prefix).is_absolute()),
        "invalid remote mirror prefix"
    );
    Ok(())
}

fn current_executable() -> Result<PinnedSessionExecutable> {
    use std::os::unix::fs::MetadataExt;
    let path = std::env::current_exe()?;
    let meta = std::fs::symlink_metadata(&path)?;
    let parent = path
        .parent()
        .context("session executable has no private parent")?;
    for dir in [
        parent,
        parent
            .parent()
            .context("session executable has no private root")?,
    ] {
        let meta = std::fs::symlink_metadata(dir)?;
        anyhow::ensure!(
            meta.is_dir()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o777 == 0o700,
            "hosted session executable directory is not private and owned"
        );
    }
    anyhow::ensure!(
        meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o500,
        "hosted session executable is not a private immutable regular file"
    );
    let executable = PinnedSessionExecutable {
        path: path
            .to_str()
            .context("session executable path is not UTF-8")?
            .into(),
        sha256: crate::toolchain::sha256(&path)?,
    };
    executable.validate()?;
    Ok(executable)
}

fn agent_command(prefix: &str) -> Result<Command> {
    check_prefix(prefix)?;
    let mut command = Command::new(std::env::current_exe()?);
    command.arg("__agent");
    if !prefix.is_empty() {
        command.args(["--prefix", prefix]);
    }
    Ok(command)
}

fn session_script(
    mode: &str,
    session: &str,
    prefix: &str,
    executable: &PinnedSessionExecutable,
) -> String {
    format!(
        "export PATH=\"$HOME/.cargo/bin:/opt/homebrew/bin:$HOME/.homebrew/bin:/usr/local/bin:$PATH\"; exec {} {ARG} {mode} {session} {}",
        crate::remote_build::shell_quote(&executable.path),
        crate::remote_build::shell_quote(prefix)
    )
}

fn broker(session: &str, prefix: &str) -> Result<()> {
    check_prefix(prefix)?;
    let executable = current_executable()?;
    let dir = session_dir(session)?;
    // create_new ownership is established by the one-shot create operation.
    let listener = UnixListener::bind(dir.join("sock"))?;
    listener.set_nonblocking(true)?;
    let identity = Identity {
        version: 1,
        session: session.into(),
        incarnation: token()?,
        machine: crate::procfs::hostname(),
        boot: format!(
            "{:?}",
            crate::procfs::boot_time().context("cannot identify host boot")?
        ),
        binary: crate::remote_daemon::build_id(),
        prefix: prefix.into(),
        executable,
    };
    save(&dir.join("session.json"), &identity)?;
    let journal = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(dir.join("output"))?;
    let mut command = agent_command(prefix)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .detached();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        let owner = std::process::id() as libc::pid_t;
        // SAFETY: prctl/getppid/_exit are the only post-fork operations.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != owner {
                    libc::_exit(128 + libc::SIGTERM);
                }
                Ok(())
            });
        }
    }
    let mut child = command.spawn()?;
    let stdout = child.stdout.take().unwrap();
    let input = child.stdin.take().unwrap();
    if let Err(error) = nonblocking(&input) {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    let broker = Arc::new(Broker {
        identity,
        dir: dir.clone(),
        input: Mutex::new(Some(input)),
        state: Mutex::new(BrokerState {
            accepted: 0,
            length: 0,
            jobs: BTreeMap::new(),
            fatal: None,
            done: false,
            inputs: BTreeMap::new(),
        }),
        changed: Condvar::new(),
    });
    let reader = {
        let broker = broker.clone();
        std::thread::spawn(move || {
            let mut stdout = std::io::BufReader::new(stdout);
            let mut journal = journal;
            loop {
                match read_frame(&mut stdout) {
                    Ok(Some((msg, data))) => {
                        if let Err(e) = broker.output(&msg, &data, &mut journal) {
                            broker.fail(e);
                            break;
                        }
                    }
                    Ok(None) => break,
                    Err(e) => {
                        broker.fail(e);
                        break;
                    }
                }
            }
            let mut state = broker.state.lock().unwrap();
            state.done = true;
            broker.changed.notify_all();
        })
    };
    let mut detached = Instant::now();
    loop {
        let state = broker.state.lock().unwrap();
        let stop = state.done || state.fatal.is_some();
        drop(state);
        if stop || detached.elapsed() >= GRACE {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                match attachment(&broker, stream) {
                    Ok(true) => break,
                    Ok(false) => {}
                    // A protocol/identity refusal never changes the broker's
                    // original job. The legitimate client can still reconnect.
                    Err(e) => eprintln!("durable session attachment: {e:#}"),
                }
                detached = Instant::now();
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(e) => {
                broker.fail(e);
                break;
            }
        }
    }
    terminate(&broker, &mut child, Duration::from_secs(5));
    let _ = reader.join();
    {
        let mut state = broker.state.lock().unwrap();
        for (id, job) in &mut state.jobs {
            if job.state == "submission_pending" {
                job.state = "lostjob".into();
                let _ = save(&dir.join(format!("job-{id}.json")), job);
            }
        }
    }
    let _ = std::fs::remove_file(dir.join("sock"));
    // Only fully reaped sessions are eligible for later private-log expiry.
    let _ = save(&dir.join("finished.json"), &true);
    Ok(())
}

/// This is not workspace/cache GC. Remove only our own finished transport
/// receipts after one day, never a live socket or an unfamiliar directory.
fn expire(parent: &Path) {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(parent) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.len() != 32 || !name.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || meta.uid() != unsafe { libc::geteuid() } || path.join("sock").exists()
        {
            continue;
        }
        let Ok(finished) = std::fs::symlink_metadata(path.join("finished.json")) else {
            continue;
        };
        if !finished.is_file()
            || finished
                .modified()
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|age| age < Duration::from_secs(24 * 3600))
        {
            continue;
        }
        let Ok(files) = std::fs::read_dir(&path) else {
            continue;
        };
        for file in files.flatten() {
            let name = file.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let job = name
                .strip_prefix("job-")
                .and_then(|n| n.strip_suffix(".json"))
                .is_some_and(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()));
            if matches!(
                name,
                "output" | "broker.log" | "session.json" | "finished.json"
            ) || job
            {
                let _ = std::fs::remove_file(file.path());
            }
        }
        let _ = std::fs::remove_dir(path);
    }
}

/// Keep Child owned and unreaped until all cancellation attempts have ended.
/// No pipe mutex acquisition or write can outlive this one graceful deadline.
fn terminate(broker: &Broker, child: &mut Child, grace: Duration) {
    let deadline = Instant::now() + grace;
    broker.cancel(deadline);
    while Instant::now() < deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        let active = broker.state.try_lock().map_or(true, |state| {
            state
                .jobs
                .values()
                .any(|job| job.state == "submission_pending")
        });
        if !active && let Ok(mut input) = broker.input.try_lock() {
            input.take();
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    // Kill and reap the exact owned agent even if its input writer is stuck.
    // Its fatal handler/PDEATHSIG continues to own the recorded build groups.
    let _ = child.kill();
    let _ = child.wait();
}

fn attachment(broker: &Arc<Broker>, mut stream: UnixStream) -> Result<bool> {
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    let (
        Packet::Attach {
            session,
            prefix,
            identity,
            offset,
        },
        data,
    ) = receive(&mut stream)?
    else {
        bail!("expected session attachment");
    };
    let state = broker.state.lock().unwrap();
    let valid = data.is_empty()
        && session == broker.identity.session
        && prefix == broker.identity.prefix
        && identity.as_ref().is_none_or(|id| id == &broker.identity)
        && offset <= state.length;
    let accepted = state.accepted;
    drop(state);
    if !valid {
        send(
            &mut stream,
            &Packet::Fatal {
                message: "session identity or output offset changed (lostjob)".into(),
            },
            &[],
        )?;
        bail!("session identity or cursor mismatch");
    }
    send(
        &mut stream,
        &Packet::Ready {
            identity: broker.identity.clone(),
            accepted,
        },
        &[],
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let disconnected = Arc::new(AtomicBool::new(false));
    let mut output = stream.try_clone()?;
    let writer = {
        let broker = broker.clone();
        let disconnected = disconnected.clone();
        std::thread::spawn(move || -> Result<()> {
            let mut journal = File::open(broker.dir.join("output"))?;
            let mut offset = offset;
            let mut acknowledged = accepted;
            let mut pulse = Instant::now();
            loop {
                let state = broker.state.lock().unwrap();
                if disconnected.load(Ordering::SeqCst) {
                    return Ok(());
                }
                if state.accepted != acknowledged || pulse.elapsed() >= Duration::from_secs(1) {
                    acknowledged = state.accepted;
                    drop(state);
                    send(&mut output, &Packet::Ack { seq: acknowledged }, &[])?;
                    pulse = Instant::now();
                    continue;
                }
                let available = state.length - offset;
                if available > 0 {
                    let n = available.min(64 << 10) as usize;
                    drop(state);
                    let mut bytes = vec![0; n];
                    journal.seek(SeekFrom::Start(offset))?;
                    journal.read_exact(&mut bytes)?;
                    send(&mut output, &Packet::Output { offset }, &bytes)?;
                    offset += n as u64;
                } else if let Some(message) = &state.fatal {
                    let message = message.clone();
                    drop(state);
                    send(&mut output, &Packet::Fatal { message }, &[])?;
                    return Ok(());
                } else if state.done {
                    drop(state);
                    send(&mut output, &Packet::End, &[])?;
                    return Ok(());
                } else {
                    let _ = broker
                        .changed
                        .wait_timeout(state, Duration::from_millis(100))
                        .unwrap();
                }
            }
        })
    };
    let mut closing = false;
    loop {
        match receive(&mut stream) {
            Ok((Packet::Input { seq }, bytes)) => {
                match broker.input(seq, &bytes) {
                    Ok(_) => {
                        // The sole output writer publishes ACKs without races.
                        broker.changed.notify_all();
                    }
                    Err(e) => {
                        broker.fail(e);
                        break;
                    }
                }
            }
            Ok((Packet::Pulse, bytes)) if bytes.is_empty() => {}
            Ok((Packet::Close, bytes)) if bytes.is_empty() => {
                closing = true;
                break;
            }
            Ok(_) => {
                broker.fail("unexpected durable-session input");
                break;
            }
            Err(_) => break,
        }
    }
    disconnected.store(true, Ordering::SeqCst);
    broker.changed.notify_all();
    let _ = stream.shutdown(std::net::Shutdown::Both);
    let _ = writer.join();
    Ok(closing)
}

fn relay(session: &str, prefix: &str, create: bool) -> Result<()> {
    check_prefix(prefix)?;
    let _executable = current_executable()?;
    let dir = session_dir(session)?;
    if create {
        let parent = dir.parent().unwrap();
        std::fs::create_dir_all(parent)?;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
        expire(parent);
        std::fs::create_dir(&dir).context("session already exists, refusing duplicate creation")?;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        let log = OpenOptions::new()
            .create_new(true)
            .append(true)
            .mode(0o600)
            .open(dir.join("broker.log"))?;
        Command::new(std::env::current_exe()?)
            .args([ARG, "serve", session, prefix])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .detached()
            .spawn()?;
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut socket = loop {
        match UnixStream::connect(dir.join("sock")) {
            Ok(socket) => break socket,
            Err(e) if create && Instant::now() < deadline => {
                let _ = e;
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => {
                // Structured refusal prevents the client treating reboot or
                // a disappeared session as a reason to start another broker.
                send(
                    &mut std::io::stdout().lock(),
                    &Packet::Fatal {
                        message: format!("original remote session is gone (lostjob): {e}"),
                    },
                    &[],
                )?;
                return Ok(());
            }
        }
    };
    let mut input_socket = socket.try_clone()?;
    let input = std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut input_socket);
        let _ = input_socket.shutdown(std::net::Shutdown::Write);
    });
    let result = copy_live(&mut socket, &mut std::io::stdout().lock());
    let _ = socket.shutdown(std::net::Shutdown::Both);
    // stdin is an SSH channel, closed by its local owner when stdout ends.
    // Do not wait forever on a blocked stdin reader after transport loss.
    drop(input);
    result?;
    Ok(())
}

struct Pending {
    frames: VecDeque<(u64, Vec<u8>)>,
    next: u64,
    bytes: usize,
    closed: bool,
    error: Option<String>,
}

fn acknowledge(pending: &mut Pending, accepted: u64) -> Result<()> {
    if accepted >= pending.next {
        bail!("remote acknowledged unsent session input");
    }
    while pending
        .frames
        .front()
        .is_some_and(|(seq, _)| *seq <= accepted)
    {
        pending.bytes -= pending.frames.pop_front().unwrap().1.len();
    }
    Ok(())
}

fn retryable(error: &anyhow::Error) -> bool {
    error.downcast_ref::<std::io::Error>().is_some_and(|e| {
        matches!(
            e.kind(),
            std::io::ErrorKind::UnexpectedEof
                | std::io::ErrorKind::BrokenPipe
                | std::io::ErrorKind::ConnectionReset
                | std::io::ErrorKind::ConnectionAborted
                | std::io::ErrorKind::TimedOut
                | std::io::ErrorKind::NotConnected
        )
    })
}

#[derive(Default)]
struct RecoveryBudget {
    retries: usize,
    lost_at: Option<Instant>,
}

impl RecoveryBudget {
    fn deadline(&self) -> Option<Instant> {
        self.lost_at.map(|at| at + GRACE)
    }

    fn retry(&mut self, error: &anyhow::Error, now: Instant) -> bool {
        if !retryable(error) {
            return false;
        }
        let started = *self.lost_at.get_or_insert(now);
        if self.retries >= RECONNECTS || now.saturating_duration_since(started) >= GRACE {
            return false;
        }
        self.retries += 1;
        true
    }
}

fn bridge(target: Target, prefix: String, executable: PinnedSessionExecutable) -> Result<()> {
    check_prefix(&prefix)?;
    executable.validate()?;
    let session = token()?;
    let pending = Arc::new((
        Mutex::new(Pending {
            frames: VecDeque::new(),
            next: 1,
            bytes: 0,
            closed: false,
            error: None,
        }),
        Condvar::new(),
    ));
    {
        let pending = pending.clone();
        std::thread::spawn(move || {
            let mut input = std::io::stdin().lock();
            loop {
                let frame = read_frame(&mut input);
                let (lock, cv) = &*pending;
                let mut queue = lock.lock().unwrap();
                match frame {
                    Ok(Some((msg, data))) => match encode(&msg, &data) {
                        Ok(bytes) => {
                            if bytes.len() > MAX_PENDING {
                                queue.error =
                                    Some("agent frame exceeds pending transport limit".into());
                                queue.closed = true;
                                cv.notify_all();
                                break;
                            }
                            while queue.bytes + bytes.len() > MAX_PENDING && queue.error.is_none() {
                                queue = cv.wait(queue).unwrap();
                            }
                            if queue.error.is_some() {
                                break;
                            }
                            let seq = queue.next;
                            queue.next += 1;
                            queue.bytes += bytes.len();
                            queue.frames.push_back((seq, bytes));
                        }
                        Err(e) => {
                            queue.error = Some(e.to_string());
                            queue.closed = true;
                        }
                    },
                    Ok(None) => queue.closed = true,
                    Err(e) => {
                        queue.error = Some(e.to_string());
                        queue.closed = true;
                    }
                }
                cv.notify_all();
                if queue.closed {
                    break;
                }
            }
        });
    }
    let mut identity: Option<Identity> = None;
    let mut offset = 0;
    let mut recovery = RecoveryBudget::default();
    let mut create = true;
    loop {
        let mode = if create { "create" } else { "attach" };
        // Creation is one-shot even when its response is lost. All later
        // attempts MUST attach, never attach-or-start.
        create = false;
        let script = session_script(mode, &session, &prefix, &executable);
        let mut child = target
            .command()
            .arg(script)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut input = child.stdin.take().unwrap();
        let mut output = child.stdout.take().unwrap();
        let pid = child.id() as i32;
        let connection_done = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicBool::new(false));
        let last_seen = Arc::new(Mutex::new(Instant::now()));
        let deadline = recovery.deadline();
        let watchdog = {
            let done = connection_done.clone();
            let connected = connected.clone();
            let last_seen = last_seen.clone();
            std::thread::spawn(move || {
                while !done.load(Ordering::SeqCst) {
                    if last_seen.lock().unwrap().elapsed() >= Duration::from_secs(10)
                        || (!connected.load(Ordering::SeqCst)
                            && deadline.is_some_and(|at| Instant::now() >= at))
                    {
                        // Only this attempt's SSH child, never its shared master.
                        unsafe {
                            libc::kill(pid, libc::SIGKILL);
                        }
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
            })
        };
        let result = (|| -> Result<bool> {
            send(
                &mut input,
                &Packet::Attach {
                    session: session.clone(),
                    prefix: prefix.clone(),
                    identity: identity.clone(),
                    offset,
                },
                &[],
            )?;
            let (ready, data) = receive(&mut output)?;
            let accepted = match ready {
                Packet::Ready {
                    identity: got,
                    accepted,
                } if data.is_empty() => {
                    if got.session != session
                        || got.prefix != prefix
                        || got.executable != executable
                        || got.version != 1
                        || identity.as_ref().is_some_and(|old| old != &got)
                    {
                        bail!("original remote session identity changed (lostjob)");
                    }
                    identity = Some(got);
                    accepted
                }
                Packet::Fatal { message } => bail!("{message}"),
                _ => bail!("invalid durable-session attachment response"),
            };
            connected.store(true, Ordering::SeqCst);
            *last_seen.lock().unwrap() = Instant::now();
            let (lock, cv) = &*pending;
            acknowledge(&mut lock.lock().unwrap(), accepted)?;
            cv.notify_all();
            let stopped = Arc::new(AtomicBool::new(false));
            let writer = {
                let pending = pending.clone();
                let stopped = stopped.clone();
                std::thread::spawn(move || -> Result<()> {
                    let (lock, cv) = &*pending;
                    let mut sent = accepted;
                    let mut pulse = Instant::now();
                    loop {
                        let queue = lock.lock().unwrap();
                        if stopped.load(Ordering::SeqCst) {
                            return Ok(());
                        }
                        if let Some(error) = &queue.error {
                            bail!("{error}");
                        }
                        if let Some((seq, bytes)) = queue.frames.iter().find(|(seq, _)| *seq > sent)
                        {
                            let seq = *seq;
                            let bytes = bytes.clone();
                            drop(queue);
                            if let Err(error) = send(&mut input, &Packet::Input { seq }, &bytes) {
                                unsafe {
                                    libc::kill(pid, libc::SIGKILL);
                                }
                                return Err(error);
                            }
                            sent = seq;
                        } else if queue.closed {
                            drop(queue);
                            send(&mut input, &Packet::Close, &[])?;
                            return Ok(());
                        } else if pulse.elapsed() >= Duration::from_secs(1) {
                            drop(queue);
                            if let Err(error) = send(&mut input, &Packet::Pulse, &[]) {
                                unsafe {
                                    libc::kill(pid, libc::SIGKILL);
                                }
                                return Err(error);
                            }
                            pulse = Instant::now();
                        } else {
                            let _ = cv.wait_timeout(queue, Duration::from_millis(100)).unwrap();
                        }
                    }
                })
            };
            let result = loop {
                let packet = receive(&mut output);
                if packet.is_ok() {
                    *last_seen.lock().unwrap() = Instant::now();
                }
                match packet {
                    Ok((Packet::Output { offset: at }, data)) if at == offset => {
                        if let Err(e) = publish(&mut std::io::stdout().lock(), &data) {
                            break Err(anyhow::anyhow!("local daemon output closed: {e}"));
                        }
                        offset += data.len() as u64;
                    }
                    Ok((Packet::Ack { seq }, data)) if data.is_empty() => {
                        if let Err(e) = acknowledge(&mut lock.lock().unwrap(), seq) {
                            break Err(e);
                        }
                        cv.notify_all();
                    }
                    Ok((Packet::End, data)) if data.is_empty() => break Ok(true),
                    Ok((Packet::Fatal { message }, _)) => break Err(anyhow::anyhow!(message)),
                    Ok(_) => {
                        break Err(anyhow::anyhow!(
                            "durable-session output offset/protocol mismatch"
                        ));
                    }
                    Err(e) => break Err(e),
                }
            };
            stopped.store(true, Ordering::SeqCst);
            cv.notify_all();
            let _ = child.kill();
            // Keep the unreaped PID reserved until its watchdog has stopped.
            let _ = writer.join();
            result
        })();
        connection_done.store(true, Ordering::SeqCst);
        let _ = watchdog.join();
        let _ = child.kill();
        let _ = child.wait();
        if pending.0.lock().unwrap().closed {
            return result.map(|_| ());
        }
        match result {
            Ok(_) => return Ok(()),
            Err(e) if recovery.retry(&e, Instant::now()) => {
                eprintln!(
                    "hosted: transport interrupted, reattaching original session ({}/{RECONNECTS})",
                    recovery.retries
                );
            }
            Err(e) => {
                bail!("hosted original-session recovery failed, build was not resubmitted: {e:#}")
            }
        }
    }
}

// Hosted callers use durable transport by default. Only the exact value "1"
// opts in explicitly. "0" or any unexpected/non-Unicode value selects the
// legacy direct transport, providing a strict rollback policy without guessing.
fn enabled_value(value: Option<&std::ffi::OsStr>) -> bool {
    value.is_none_or(|value| value == "1")
}

pub fn enabled() -> bool {
    enabled_value(std::env::var_os("JUSTRUST_HOSTED_SESSION").as_deref())
}

pub fn command(target: &Target, prefix: &str) -> Result<Command> {
    check_prefix(prefix)?;
    let executable = crate::remote_build::ensure_session_executable(target)?;
    command_with_executable(target, prefix, &executable)
}

fn command_with_executable(
    target: &Target,
    prefix: &str,
    executable: &PinnedSessionExecutable,
) -> Result<Command> {
    check_prefix(prefix)?;
    executable.validate()?;
    let config = serde_json::json!({ "dest": target.dest, "opts": target.opts, "prefix": prefix, "executable": executable });
    let mut command = Command::new(std::env::current_exe()?);
    command.args([ARG, "client", &config.to_string()]);
    Ok(command)
}

#[cfg(test)]
#[path = "remote_session_tests.rs"]
mod tests;

pub fn main() -> ! {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let result = (|| -> Result<()> {
        match args.as_slice() {
            [mode, config] if mode == "client" => {
                let config: serde_json::Value = serde_json::from_str(config)?;
                bridge(
                    Target {
                        dest: serde_json::from_value(config["dest"].clone())?,
                        opts: serde_json::from_value(config["opts"].clone())?,
                    },
                    serde_json::from_value(config["prefix"].clone())?,
                    serde_json::from_value(config["executable"].clone())?,
                )
            }
            [mode, session, prefix] if mode == "create" => relay(session, prefix, true),
            [mode, session, prefix] if mode == "attach" => relay(session, prefix, false),
            [mode, session, prefix] if mode == "serve" => broker(session, prefix),
            _ => bail!("invalid internal durable-session command"),
        }
    })();
    if let Err(error) = result {
        eprintln!("justrust hosted session: {error:#}");
        std::process::exit(1);
    }
    std::process::exit(0)
}
