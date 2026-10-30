//! pretty_assertions formats with yansi's `Paint` in scope, a trait every type implements that
//! has a `new`. `format_args!` ends in `<fmt::Arguments>::new(..)`, which is behind a feature:
//! the trait's stable `new` was taken for it, and nothing that formats there had a body.

use std::fmt::{self, Write};

pub trait Paint {
    fn new(self) -> Option<Self>
    where
        Self: Sized,
    {
        Some(self)
    }
}

impl<T: ?Sized> Paint for T {}

fn line<W: Write>(to: &mut W, left: &str, right: u32) -> fmt::Result {
    write!(to, "{left}|{right:>4}")
}

pub fn run() -> (String, bool) {
    let mut text = String::new();
    line(&mut text, "a", 7).unwrap();
    (text, 3u8.new().is_some())
}
