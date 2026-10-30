//! Reductions that need the 2015 edition: what later editions no longer accept. Each is run
//! through the module of the same name in `breakers`.

#![allow(ellipsis_inclusive_range_patterns)]

/// autocfg writes inclusive range patterns with three dots.
pub mod dots_range_pattern {
    fn mangle(s: &str) -> String {
        s.chars()
            .map(|c| match c {
                'A'...'Z' | 'a'...'z' | '0'...'9' => c,
                _ => '_',
            })
            .collect()
    }

    fn class(n: u8) -> u8 {
        match n {
            0...9 => 0,
            10...99 => 1,
            _ => 2,
        }
    }

    pub fn run() -> (String, [u8; 4]) {
        (mangle("has-atomic 64!"), [class(0), class(9), class(10), class(200)])
    }
}

/// version-sync 0.8 panics with a `String` that is not a format string, which editions
/// before 2021 take as the panic's payload.
pub mod panic_with_string {
    fn check(path: &str) -> Result<(), String> {
        Err(format!("could not read {}", path))
    }

    fn checked() {
        if let Err(err) = check("no/such/file.md") {
            panic!(err);
        }
    }

    pub fn run() -> (String, i32) {
        let text = ::std::panic::catch_unwind(checked).unwrap_err();
        let number = ::std::panic::catch_unwind(|| panic!(7)).unwrap_err();
        (
            text.downcast_ref::<String>().cloned().unwrap_or_default(),
            number.downcast_ref::<i32>().cloned().unwrap_or_default(),
        )
    }
}

/// plain's tests import the crate root with `use ::*;` and name its private modules.
pub mod glob_of_root {
    use ::*;

    pub fn run() -> u32 {
        doubling::double(21)
    }
}

mod doubling {
    pub fn double(n: u32) -> u32 {
        n * 2
    }
}

/// fxhash's trait for hashing a word leaves its parameter unnamed.
pub mod unnamed_parameter {
    trait HashWord {
        fn hash_word(&mut self, Self);
    }

    impl HashWord for u32 {
        fn hash_word(&mut self, word: Self) {
            *self = self.rotate_left(5) ^ word;
        }
    }

    fn hashed(mut hash: u32, bytes: &[u8]) -> u32 {
        for byte in bytes {
            hash.hash_word(*byte as u32);
        }
        hash
    }

    pub fn run() -> u32 {
        hashed(0, b"plain words")
    }
}

/// lewton 0.10 writes its own `try!` through `$crate::std`, and rodio 0.22 reaches it from
/// bevy_audio: in a 2015 crate the root holds the `extern crate std;` rustc injects, so
/// `$crate::std`, `crate::std` and `super::std` name it.
pub mod std_in_root {
    macro_rules! attempt {
        ($expr:expr) => {
            match $expr {
                $crate::std::result::Result::Ok(val) => val,
                $crate::std::result::Result::Err(err) => {
                    return Err($crate::std::convert::From::from(err));
                }
            }
        };
    }

    fn digit(c: char) -> Result<u32, char> {
        c.to_digit(10).ok_or(c)
    }

    fn sum(text: &str) -> Result<u32, String> {
        let mut total = 0;
        for c in text.chars() {
            total += attempt!(digit(c).map_err(|c| format!("not a digit: {}", c)));
        }
        Ok(total)
    }

    fn named(text: &str) -> crate::std::string::String {
        let parts: super::std::vec::Vec<&str> = text.split(',').collect();
        parts.join("+")
    }

    pub fn run() -> (Result<u32, String>, Result<u32, String>, String) {
        (sum("1234"), sum("12x4"), named("a,b,c"))
    }
}

use std::panic::catch_unwind;

/// What a panic that unwound said, when it said a string.
pub fn said(payload: Box<dyn std::any::Any + Send>) -> String {
    match payload.downcast::<String>() {
        Ok(said) => *said,
        Err(payload) => payload.downcast::<&str>().map(|said| said.to_string()).unwrap_or_default(),
    }
}

/// tests/ui/fmt/format-args-capture.rs: before the 2021 edition a `panic!` of one string says
/// that string as it is, braces and all, and so does one an `assert!` makes.
#[inline(never)]
pub fn braces_stay(low: u8) -> [String; 2] {
    let alone = catch_unwind(|| panic!("{braces}"));
    let asserted = catch_unwind(|| assert!(low > 200, "{braces}"));
    [alone.map_err(said).unwrap_err(), asserted.map_err(said).unwrap_err()]
}

/// A `panic!` written by a macro of this edition, wherever it is called from.
#[macro_export]
macro_rules! panic_with_braces {
    () => {
        panic!("{braces}")
    };
}
