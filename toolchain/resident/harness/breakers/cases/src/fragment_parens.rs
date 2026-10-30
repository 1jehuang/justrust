//! darling_core hands an `expr` fragment on to `quote!`, which takes a parenthesized group
//! apart. To rustc the fragment is a group without delimiters and matches only as one token
//! tree; here it is passed on in parentheses.

macro_rules! shape {
    (($($inner:tt)*)) => {
        "parentheses"
    };
    ($other:tt) => {
        "one tree"
    };
}

macro_rules! pass {
    ($body:expr) => {
        shape!($body)
    };
}

pub fn run() -> [&'static str; 3] {
    [pass!("World" + 5), pass!((1, 2)), pass!(7)]
}
