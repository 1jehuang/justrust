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

/// Result of `main` reported to the runtime (stock `std::process::Termination`).
pub trait Termination {
    fn report(self) -> ExitCode;
}

/// Process exit status returned from `main` (stock `std::process::ExitCode`).
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ExitCode {
    code: u8,
}

impl super::convert::From<u8> for ExitCode {
    fn from(code: u8) -> ExitCode {
        ExitCode { code }
    }
}

impl Termination for () {
    fn report(self) -> ExitCode {
        ExitCode { code: 0u8 }
    }
}

impl Termination for ExitCode {
    fn report(self) -> ExitCode {
        self
    }
}

impl<T: Termination, E: super::fmt::Debug> Termination for super::result::Result<T, E> {
    fn report(self) -> ExitCode {
        match self {
            super::result::Result::Ok(value) => value.report(),
            super::result::Result::Err(error) => {
                eprintln!("Error: {:?}", error);
                ExitCode { code: 1u8 }
            }
        }
    }
}

/// Entry adapter for a non-unit `main` (stock `lang_start`): a nonzero
/// report exits with that status; destructors of `main` already ran.
pub fn __justrust_report<T: Termination>(result: T) {
    let code = result.report().code;
    if code != 0u8 {
        exit(code as i32);
    }
}
