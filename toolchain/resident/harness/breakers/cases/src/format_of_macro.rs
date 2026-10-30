//! anes writes `write!(f, csi!("foo"))`: the format string is what a macro expands to, a
//! `concat!` of literals.

use std::fmt;

macro_rules! csi {
    ($($arg:expr),*) => { concat!("\x1B[", $($arg),*) };
}

macro_rules! sequence {
    (struct $name:ident => $value:expr) => {
        struct $name;

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
                write!(f, $value)
            }
        }
    };
}

sequence!(struct Reset => csi!("0m"));

pub fn run() -> (String, String, &'static str) {
    sequence!(struct Inner => csi!("foo"));
    (format!("{Inner}"), Reset.to_string(), csi!("bar"))
}
