//! tests/ui/fmt/format-args-capture.rs: `panic!("{foo}")` is a format string from the 2021
//! edition on, and before it the string itself. The edition is that of where the `panic!` is
//! written, or of the macro that wrote it; a macro of the standard library (`assert!`) stands
//! for where it is called. ra-jit asked the edition of the innermost macro, so a 2015 crate
//! was left to rustc looking for a variable named in the braces.

use std::panic::catch_unwind;

use breakers2015::{braces_stay, said};

pub fn run() -> ([String; 2], String, String) {
    let braces = 7;
    let through_macro = catch_unwind(|| breakers2015::panic_with_braces!());
    let here = catch_unwind(|| assert!(braces > 200, "{braces}"));
    (braces_stay(braces), through_macro.map_err(said).unwrap_err(), here.map_err(said).unwrap_err())
}
