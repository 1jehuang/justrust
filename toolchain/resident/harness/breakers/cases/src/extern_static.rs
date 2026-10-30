//! onig reads the encodings of its C library as `extern` statics. A static declared in an
//! `extern` block is defined by whatever is linked, under its own name, whoever compiles
//! the crate that declares it.

#[cfg(unix)]
use std::ffi::c_char;
use std::ffi::c_int;

#[cfg(unix)]
unsafe extern "C" {
    static environ: *const *const c_char;
    #[link_name = "optind"]
    static mut NEXT_ARGUMENT: c_int;
}

#[cfg(unix)]
pub fn run() -> (bool, c_int) {
    let has_environment = unsafe { !environ.is_null() };
    let next = unsafe { *(&raw const NEXT_ARGUMENT) };
    (has_environment, next)
}

/// The C library of Windows has neither as a symbol.
#[cfg(not(unix))]
pub fn run() -> (bool, c_int) {
    (true, 1)
}
