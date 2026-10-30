//! num_enum asserts on a long chain of calls. rustc prints the expression of a failed
//! `assert!` with its pretty printer, which wraps lines and drops the comma after the last
//! element of an array.

use std::panic::catch_unwind;

fn message(check: fn()) -> String {
    let payload = catch_unwind(check).expect_err("the assertion fails");
    payload.downcast_ref::<&str>().map(|it| it.to_string()).unwrap_or_default()
}

fn short() {
    assert!([1, 2,].len() == 3);
}

fn long() {
    assert!(String::from("a rather long first string literal")
        .chars()
        .chain(["second element of the array", "third element of the array",].concat().chars())
        .filter(|it| it.is_ascii_uppercase())
        .count()
        == 1);
}

pub fn run() -> (String, String) {
    (message(short), message(long))
}
