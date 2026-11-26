//! Test-only waits for the fork window of a multithreaded test binary.
//!
//! Between another test thread's fork and its exec, the child holds a copy of
//! every descriptor in this process. A file this test just wrote stays open
//! for writing there, so exec of it fails with ETXTBSY, and a lock this test
//! just dropped stays held. Both end when that child execs. A test that runs
//! what it wrote, or relocks what it released, waits the window out here
//! rather than failing and being rerun alone.

use std::time::{Duration, Instant};

const WINDOW: Duration = Duration::from_secs(5);
const STEP: Duration = Duration::from_millis(10);

/// Run `f`, which execs a file this test just wrote. Only ETXTBSY is retried.
pub(crate) fn exec_fresh<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    retry(std::io::ErrorKind::ExecutableFileBusy, &mut f)
}

/// Run `f`, a command that is, or is a shell script that execs, a file this
/// test just wrote. The shell reports the busy file itself, as exit code 126
/// (found but not executable), so that is retried too.
pub(crate) fn output_fresh(
    mut f: impl FnMut() -> std::io::Result<std::process::Output>,
) -> std::io::Result<std::process::Output> {
    let deadline = Instant::now() + WINDOW;
    loop {
        let out = exec_fresh(&mut f)?;
        if out.status.code() != Some(126) || Instant::now() >= deadline {
            return Ok(out);
        }
        std::thread::sleep(STEP);
    }
}

/// Run `f`, which takes a non-blocking lock this test released a moment ago.
/// Only a held lock (EWOULDBLOCK) is retried.
pub(crate) fn relock<T>(mut f: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    retry(std::io::ErrorKind::WouldBlock, &mut f)
}

fn retry<T>(
    kind: std::io::ErrorKind,
    f: &mut dyn FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    let deadline = Instant::now() + WINDOW;
    loop {
        match f() {
            Err(e) if e.kind() == kind && Instant::now() < deadline => std::thread::sleep(STEP),
            result => return result,
        }
    }
}

/// Whether `f` becomes true within the fork window.
pub(crate) fn eventually(mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + WINDOW;
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(STEP);
    }
}

#[test]
fn only_a_busy_executable_is_retried() {
    let mut calls = 0;
    let busy = || std::io::Error::from(std::io::ErrorKind::ExecutableFileBusy);
    let out = exec_fresh(|| {
        calls += 1;
        if calls < 3 { Err(busy()) } else { Ok(calls) }
    });
    assert_eq!(out.unwrap(), 3);

    let mut calls = 0;
    let out: std::io::Result<()> = exec_fresh(|| {
        calls += 1;
        Err(std::io::ErrorKind::NotFound.into())
    });
    assert_eq!(out.unwrap_err().kind(), std::io::ErrorKind::NotFound);
    assert_eq!(calls, 1, "other errors are reported at once");

    let mut calls = 0;
    let out = relock(|| {
        calls += 1;
        if calls < 2 {
            Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
        } else {
            Ok(calls)
        }
    });
    assert_eq!(out.unwrap(), 2);
    let out: std::io::Result<()> = relock(|| Err(busy()));
    assert_eq!(
        out.unwrap_err().kind(),
        std::io::ErrorKind::ExecutableFileBusy
    );

    let mut polls = 0;
    assert!(eventually(|| {
        polls += 1;
        polls == 2
    }));
}
