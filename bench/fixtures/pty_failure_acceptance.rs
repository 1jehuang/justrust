// Acceptance-only failure injection tests, never applied to benchmark sources.
// The acceptance generator instruments the child branch for these named cases.
fn assert_fault_is_reaped(expected: &str) {
    let start = std::time::Instant::now();
    let error = std::panic::catch_unwind(|| decode_key_event_via_pty(b"\r"))
        .expect_err("injected broken child must fail, not skip");
    let message = error.downcast_ref::<String>().map(String::as_str)
        .or_else(|| error.downcast_ref::<&str>().copied()).unwrap_or("");
    assert!(message.contains(expected), "wrong fault: {message}");
    assert!(start.elapsed() < std::time::Duration::from_secs(3));
    let mut status = 0;
    assert_eq!(unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) }, -1,
        "guard must reap its child even on panic");
    assert_eq!(std::io::Error::last_os_error().raw_os_error(), Some(libc::ECHILD));
}
#[test]
fn fault_timeout() { assert_fault_is_reaped("protocol deadline exceeded"); }
#[test]
fn fault_malformed() { assert_fault_is_reaped("enable raw mode before input"); }
#[test]
fn fault_exit() { assert_fault_is_reaped("PTY child failed"); }
