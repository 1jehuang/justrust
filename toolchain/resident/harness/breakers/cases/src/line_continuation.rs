//! litrs compares what it parses with string literals that end in a line continuation,
//! which gives no character after it, and that skip more than one line, which the compiler
//! only warns about.

use std::ffi::CStr;

type Literals = (&'static str, &'static [u8], &'static CStr, &'static str, &'static [u8]);

#[allow(warnings)]
pub fn run() -> Literals {
    let nothing = "\
";
    let lines = b"foo\

        banana";
    let c_nothing = c"\
";
    let end = "end \
    ";
    let tail = b"tail\
";
    (nothing, lines, c_nothing, end, tail)
}
