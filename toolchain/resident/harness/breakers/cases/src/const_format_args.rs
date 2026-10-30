//! equator keeps `const { &format_args!("") }` for an assertion without a message and asks
//! whether it is the empty string before it formats it. What a `const` block borrows lives
//! as long as the program does.

use std::fmt::Arguments;

fn empty<'a>() -> &'a Arguments<'a> {
    const { &format_args!("") }
}

fn fixed<'a>() -> &'a Arguments<'a> {
    const { &format_args!("no message") }
}

fn pair() -> &'static (u8, Option<&'static str>) {
    const { &(7, Some("seven")) }
}

pub fn run() -> (Option<&'static str>, Option<&'static str>, String, String, (u8, Option<&'static str>)) {
    (empty().as_str(), fixed().as_str(), format!("{:?}", empty()), format!("<{}>", fixed()), *pair())
}
