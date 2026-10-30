//! The lines `cargo ra-jit` prints itself, in cargo's shape: a status word right-aligned in
//! twelve columns, on standard error.

use std::io::IsTerminal as _;
use std::sync::atomic::{AtomicU8, Ordering};

const GREEN: &str = "\x1b[1m\x1b[92m";
const YELLOW: &str = "\x1b[1m\x1b[93m";
const RED: &str = "\x1b[1m\x1b[91m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// What the command's `--color` said: [`AUTO`], [`ALWAYS`] or [`NEVER`].
static COLOR: AtomicU8 = AtomicU8::new(AUTO);
const AUTO: u8 = 0;
const ALWAYS: u8 = 1;
const NEVER: u8 = 2;

/// Has the lines colored as `when` says, cargo's `--color`, or else as `CARGO_TERM_COLOR`
/// does: `always`, `never`, or `auto`, when standard error is a terminal.
pub(crate) fn color(when: Option<&str>) {
    let when = when.map(str::to_owned).or_else(|| std::env::var("CARGO_TERM_COLOR").ok());
    let when = match when.as_deref() {
        Some("always") => ALWAYS,
        Some("never") => NEVER,
        _ => AUTO,
    };
    COLOR.store(when, Ordering::Relaxed);
}

/// Whether the lines are colored.
pub(crate) fn colored() -> bool {
    match COLOR.load(Ordering::Relaxed) {
        ALWAYS => true,
        NEVER => false,
        _ => std::io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    }
}

/// Whether what goes to standard output is colored: as [`colored`], with standard output's
/// terminal for `auto`.
pub(crate) fn colored_out() -> bool {
    match COLOR.load(Ordering::Relaxed) {
        ALWAYS => true,
        NEVER => false,
        _ => std::io::stdout().is_terminal() && std::env::var_os("NO_COLOR").is_none(),
    }
}

fn painted(color: &str, word: &str) -> String {
    match colored() {
        true => format!("{color}{word}{RESET}"),
        false => word.to_owned(),
    }
}

/// `     Running target/debug/app`, unless the build is quiet.
pub(crate) fn status(quiet: bool, word: &str, rest: &str) {
    if !quiet {
        eprintln!("{} {rest}", painted(GREEN, &format!("{word:>12}")));
    }
}

/// Why cargo does what the daemon would have. Never quiet: a build that went another way
/// than asked says so.
pub(crate) fn fallback(reason: &str, then: &str) {
    eprintln!("{} {reason}; {then}", painted(YELLOW, "blaze:"));
}

pub(crate) fn error(text: &str) {
    eprintln!("{}: {text}", painted(RED, "error"));
}

/// [`error`] with no newline after it, as cargo ends a few of clap's.
pub(crate) fn error_unended(text: &str) {
    eprint!("{}: {text}", painted(RED, "error"));
}

pub(crate) fn warning(text: &str) {
    eprintln!("{}: {text}", painted(YELLOW, "warning"));
}

pub(crate) fn note(text: &str) {
    eprintln!("{}: {text}", painted(BOLD, "note"));
}

/// A note as cargo's shell says one (`Shell::note`), green, which a diagnostic's is not.
pub(crate) fn shell_note(text: &str) {
    eprintln!("{}: {text}", painted(GREEN, "note"));
}
