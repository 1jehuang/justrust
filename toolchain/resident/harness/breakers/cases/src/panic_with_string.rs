//! version-sync 0.8 panics with a `String` that is not a format string, which editions before
//! 2021 take as the panic's payload.

pub fn run() -> (String, i32) {
    breakers2015::panic_with_string::run()
}
