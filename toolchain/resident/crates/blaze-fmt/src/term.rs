//! The few calls rustfmt makes into the `term` crate, answered with ANSI escapes. rustfmt colours
//! whenever terminfo knows `$TERM`, pipes included; this colours only a terminal, so piped or
//! captured output (`--check` diffs in CI) is plain text.

use std::io::{self, IsTerminal, Write};

pub(crate) mod color {
    pub(crate) type Color = u8;

    pub(crate) const RED: Color = 1;
    pub(crate) const GREEN: Color = 2;
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Attr {
    Bold,
}

pub(crate) trait Terminal: Write {
    type Output: Write;

    fn fg(&mut self, color: color::Color) -> io::Result<()>;
    fn reset(&mut self) -> io::Result<()>;
    fn supports_color(&self) -> bool;
    fn supports_attr(&self, attr: Attr) -> bool;
}

pub(crate) struct AnsiTerminal<T: Write> {
    out: T,
}

impl<T: Write> Write for AnsiTerminal<T> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.out.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

impl<T: Write> Terminal for AnsiTerminal<T> {
    type Output = T;

    fn fg(&mut self, color: color::Color) -> io::Result<()> {
        write!(self.out, "\x1b[3{color}m")
    }

    fn reset(&mut self) -> io::Result<()> {
        write!(self.out, "\x1b[0m")
    }

    fn supports_color(&self) -> bool {
        true
    }

    fn supports_attr(&self, attr: Attr) -> bool {
        match attr {
            Attr::Bold => true,
        }
    }
}

fn colour_allowed() -> bool {
    std::env::var_os("TERM").is_some_and(|term| term != "dumb")
}

pub(crate) fn stdout() -> Option<Box<dyn Terminal<Output = io::Stdout>>> {
    let out = io::stdout();
    (colour_allowed() && out.is_terminal())
        .then(|| Box::new(AnsiTerminal { out }) as Box<dyn Terminal<Output = io::Stdout>>)
}

pub(crate) fn stderr() -> Option<Box<dyn Terminal<Output = io::Stderr>>> {
    let out = io::stderr();
    (colour_allowed() && out.is_terminal())
        .then(|| Box::new(AnsiTerminal { out }) as Box<dyn Terminal<Output = io::Stderr>>)
}
