//! Engine-owned two-phase dispatch. Only an explicit miss permits fallback.
//! Commitment is published on a private inherited FD before daemon dispatch.

use std::{
    fs::File,
    io::Write as _,
    sync::{Mutex, OnceLock},
};

use anyhow::Context as _;

#[derive(Default)]
struct State {
    ready: bool,
    prewarm: bool,
    committed: bool,
    missed: bool,
    protocol: Option<File>,
}

static STATE: OnceLock<Mutex<State>> = OnceLock::new();

fn state() -> std::sync::MutexGuard<'static, State> {
    STATE.get_or_init(Mutex::default).lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `jr-engine cargo hybrid-ready -- <command> ...`. An inherited private FD,
/// never stdout or stderr, carries the sole fallback authorization.
pub(super) fn arguments(args: Vec<String>) -> anyhow::Result<Vec<String>> {
    if !crate::jr_engine::is_engine() {
        return Ok(args);
    }
    match args.first().map(String::as_str) {
        Some("hybrid-ready") => {
            let fd = std::env::var("JR_HYBRID_DECISION_FD")
                .context("hybrid-ready requires JR_HYBRID_DECISION_FD")?
                .parse::<i32>()?;
            let protocol = protocol_file(fd)?;
            let mut state = state();
            state.ready = true;
            state.protocol = Some(protocol);
            anyhow::ensure!(
                args.get(1).map(String::as_str) == Some("--"),
                "hybrid-ready requires -- before the cargo command"
            );
            Ok(args.into_iter().skip(2).collect())
        }
        Some("hybrid-prewarm") => {
            state().prewarm = true;
            anyhow::ensure!(
                args.get(1).map(String::as_str) == Some("--"),
                "hybrid-prewarm requires -- before the cargo command"
            );
            Ok(args.into_iter().skip(2).collect())
        }
        _ => Ok(args),
    }
}

#[cfg(unix)]
fn protocol_file(fd: i32) -> anyhow::Result<File> {
    use std::os::fd::FromRawFd as _;
    anyhow::ensure!(fd >= 3, "decision fd must not alias stdin, stdout or stderr");
    // SAFETY: fcntl validates the inherited descriptor without accessing memory.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFD) };
    anyhow::ensure!(flags >= 0, "decision fd is not open");
    // SAFETY: only descriptor flags change. Daemons and workers must not inherit
    // the receipt channel or prevent the parent from observing EOF.
    anyhow::ensure!(
        unsafe { libc::fcntl(fd, libc::F_SETFD, flags | libc::FD_CLOEXEC) } >= 0,
        "cannot mark decision fd close-on-exec"
    );
    // SAFETY: the explicit inherited fd transfers ownership to this command.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(not(unix))]
fn protocol_file(_fd: i32) -> anyhow::Result<File> {
    anyhow::bail!("hybrid decision descriptors require Unix")
}

pub(super) fn ready_only() -> bool {
    state().ready
}
pub(super) fn prewarm() -> bool {
    state().prewarm
}
fn committed() -> bool {
    state().committed
}

pub(super) fn decision(commit: bool) -> anyhow::Result<()> {
    decide(&mut state(), commit)
}

fn decide(state: &mut State, commit: bool) -> anyhow::Result<()> {
    anyhow::ensure!(!state.committed, "cannot change a committed hybrid decision");
    if state.missed {
        anyhow::ensure!(!commit, "cannot commit after reporting a hybrid miss");
        return Ok(());
    }
    // Mark before writing. A partially written commitment must never become a miss.
    state.committed = commit;
    let receipt: &[u8] = if commit { b"committed\n" } else { b"miss\n" };
    let mut protocol = state.protocol.take().context("hybrid decision channel is missing")?;
    protocol.write_all(receipt)?;
    drop(protocol);
    state.missed = !commit;
    Ok(())
}

/// No daemon can execute before our commit acknowledgement. Therefore every
/// precommit error can safely become a miss, including lost admission IPC.
pub(super) fn finish(result: anyhow::Result<i32>) -> anyhow::Result<i32> {
    if ready_only() && !committed() {
        decision(false)?;
        return Ok(0);
    }
    result
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        io::Read as _,
        os::{
            fd::{AsRawFd as _, IntoRawFd as _},
            unix::net::UnixStream,
        },
        time::Duration,
    };

    fn channel() -> (State, UnixStream) {
        let (reader, writer) = UnixStream::pair().unwrap();
        reader.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        let protocol = protocol_file(writer.into_raw_fd()).unwrap();
        (State { ready: true, protocol: Some(protocol), ..State::default() }, reader)
    }

    #[test]
    fn miss_is_exact_closed_and_idempotent() {
        let (mut state, mut reader) = channel();
        decide(&mut state, false).unwrap();
        let mut receipt = String::new();
        reader.read_to_string(&mut receipt).unwrap();
        assert_eq!(receipt, "miss\n");
        assert!(state.protocol.is_none(), "EOF must not depend on dropping command state");
        decide(&mut state, false).unwrap();
        assert!(decide(&mut state, true).is_err());
        assert!(!state.committed);
    }

    #[test]
    fn commitment_is_exact_closed_and_irreversible() {
        let (mut state, mut reader) = channel();
        decide(&mut state, true).unwrap();
        let mut receipt = String::new();
        reader.read_to_string(&mut receipt).unwrap();
        assert_eq!(receipt, "committed\n");
        assert!(state.protocol.is_none());
        assert!(state.committed);
        assert!(decide(&mut state, false).is_err());
        assert!(decide(&mut state, true).is_err());
    }

    #[test]
    fn lost_commit_channel_never_becomes_a_miss() {
        let (mut state, reader) = channel();
        drop(reader);
        assert!(decide(&mut state, true).is_err());
        assert!(state.committed);
        assert!(decide(&mut state, false).is_err());
    }

    #[test]
    fn descriptor_is_close_on_exec_and_cannot_be_stdio() {
        for fd in [-1, 0, 1, 2, i32::MAX] {
            assert!(protocol_file(fd).is_err());
        }
        let (state, _reader) = channel();
        let fd = state.protocol.as_ref().unwrap().as_raw_fd();
        // SAFETY: the file owns fd throughout this descriptor-only query.
        assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFD) } & libc::FD_CLOEXEC, 0);
    }
}
