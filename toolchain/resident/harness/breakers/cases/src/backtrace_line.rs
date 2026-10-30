//! backtrace's tests resolve their own frames and expect a file and a line for each.

use std::backtrace::Backtrace;

#[inline(never)]
fn capture() -> String {
    Backtrace::force_capture().to_string()
}

/// Whether the frame of `capture` is printed with where it is in this file.
pub fn run() -> bool {
    let trace = capture();
    let mut lines = trace.lines().skip_while(|it| !it.contains("backtrace_line::capture"));
    lines.nth(1).is_some_and(|it| it.contains("backtrace_line.rs:"))
}
