//! `println!` whose template is a macro: the newline is appended to what the macro expands to.

macro_rules! report {
    ($fmt:literal, $($arg:expr),*) => {
        println!(concat!("[report]: ", $fmt), $($arg),*)
    };
}

macro_rules! template {
    () => {
        "{} then {}"
    };
}

pub fn run() {
    report!("path {:?} is missing", "fake");
    report!("{} of {}", 1, 2);
    println!(concat!("no ", "arguments"));
    println!(template!(), "one", "two");
    let text = format!(concat!("{}", "-", "{}"), 3, 4);
    println!("{text}|");
}
