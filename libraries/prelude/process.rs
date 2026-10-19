// std::process, as ordinary Rust source over the raw syscall primitive.

/// Terminate the process with `code` (exit_group). Output is unbuffered in
/// this runtime, so nothing needs flushing. Destructors do not run.
pub fn exit(code: i32) -> ! {
    let _ = unsafe {
        __justrust_linux_syscall6!(
            231usize,
            code as usize,
            0usize,
            0usize,
            0usize,
            0usize,
            0usize
        )
    };
    loop {}
}

/// Abnormal termination: SIGABRT via kill(getpid(), SIGABRT), then exit 134.
pub fn abort() -> ! {
    let pid = unsafe { __justrust_linux_syscall6!(39usize, 0usize, 0usize, 0usize, 0usize, 0usize, 0usize) };
    let _ = unsafe { __justrust_linux_syscall6!(62usize, pid, 6usize, 0usize, 0usize, 0usize, 0usize) };
    exit(134)
}
