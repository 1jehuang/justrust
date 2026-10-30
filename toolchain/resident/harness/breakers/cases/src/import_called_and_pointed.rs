//! A call names its callee in place and leaves it to the linker to go through a stub when
//! the callee is another library's, while the address of a function is still read from the
//! slot the loader fills. Both have to arrive at the same function, whether it is one of
//! this crate, of a crate rustc compiled, or of the C library, and its address has to be
//! the same wherever it is taken.

use std::ffi::{CStr, c_char, c_int};

unsafe extern "C" {
    fn strlen(text: *const c_char) -> usize;
    fn abs(value: c_int) -> c_int;
}

#[inline(never)]
fn double(value: u32) -> u32 {
    value * 2
}

#[inline(never)]
fn generic<T: Clone>(value: &T) -> (T, T) {
    (value.clone(), value.clone())
}

#[inline(never)]
fn address_here() -> usize {
    strlen as unsafe extern "C" fn(*const c_char) -> usize as usize
}

#[inline(never)]
fn address_there() -> usize {
    let pointer: unsafe extern "C" fn(*const c_char) -> usize = strlen;
    std::hint::black_box(pointer) as usize
}

fn apply(with: fn(u32) -> u32, value: u32) -> u32 {
    with(value)
}

pub fn run() -> (usize, usize, c_int, c_int, u32, u32, bool, bool, (String, String), usize) {
    let text: &CStr = c"called in place";
    let measure: unsafe extern "C" fn(*const c_char) -> usize = strlen;
    let positive: unsafe extern "C" fn(c_int) -> c_int = abs;
    let parse: fn(&str) -> Result<u32, std::num::ParseIntError> = str::parse::<u32>;
    (
        unsafe { strlen(text.as_ptr()) },
        unsafe { measure(text.as_ptr()) },
        unsafe { abs(-7) },
        unsafe { positive(-9) },
        double(21),
        apply(double, 4),
        address_here() == address_there(),
        double as fn(u32) -> u32 as usize == std::hint::black_box(double as fn(u32) -> u32) as usize,
        generic(&String::from("twice")),
        parse("12").map_or(0, |it| it as usize) + String::from("abc").len(),
    )
}
