//! An `$e:expr` that a `macro_rules!` macro hands to a procedural macro is a group without
//! delimiters to rustc, also when the expression is one token. The procedural macro sees that
//! group (const_format's `formatcp!("{}", NAME)` relies on it).

macro_rules! hand_on {
    ($e:expr) => {
        passes::fragment!($e)
    };
}

const NAME: &str = "name";

pub fn handed_on() -> (&'static str, i32, &'static str) {
    (hand_on!(NAME), hand_on!(1 + 2), hand_on!("text"))
}
