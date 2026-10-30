//! tests/ui/abi/variadic-ffi.rs: a `VaList` handed to a C function goes by its address, as
//! C's `va_list` does where it is an array, though Rust has it by value. `core` marks the
//! struct `#[rustc_pass_indirectly_in_non_rustic_abis]` for that, which ra-jit did not read: it
//! copied the list onto the stack, and the C function read its second register as the address.

use std::ffi::{c_char, c_int, CStr, VaList};

// The C runtime of MSVC has `vsnprintf` inline in its headers, and out of line in this library.
#[cfg_attr(target_env = "msvc", link(name = "legacy_stdio_definitions"))]
unsafe extern "C" {
    fn vsnprintf(buf: *mut c_char, len: usize, format: *const c_char, args: VaList<'_>) -> c_int;
}

#[inline(never)]
unsafe extern "C" fn print(buf: *mut c_char, len: usize, format: *const c_char, args: ...) -> c_int {
    unsafe { vsnprintf(buf, len, format, args) }
}

#[inline(never)]
unsafe extern "C" fn print_rest(buf: *mut c_char, len: usize, mut args: ...) -> c_int {
    let format = unsafe { args.next_arg::<*const c_char>() };
    unsafe { vsnprintf(buf, len, format, args) }
}

pub fn run() -> (String, c_int, String, c_int) {
    let (mut first, mut second) = ([0 as c_char; 64], [0 as c_char; 64]);
    unsafe {
        let wrote = print(first.as_mut_ptr(), 64, c"%d %.2f %s %ld".as_ptr(), 7i32, 2.5f64, c"x".as_ptr(), -9i64);
        let rest = print_rest(second.as_mut_ptr(), 64, c"%g %g %d".as_ptr(), 0.5f64, 4.0f64, 3i32);
        (
            CStr::from_ptr(first.as_ptr()).to_string_lossy().into_owned(),
            wrote,
            CStr::from_ptr(second.as_ptr()).to_string_lossy().into_owned(),
            rest,
        )
    }
}
