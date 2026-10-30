#!/usr/bin/env python3
"""movedlines.py: a panic reports the line it is on after lines are added above it, and the
function it is in is not compiled again for that.

A `core::panic::Location` is filled in when the object is stitched, from where the function's
body is in its file now ([`places.rs`]), so moving a function changes no compiled code. The
program panics in one function in every way that reports a place (`panic!`, `unwrap`, an index,
a `#[track_caller]` function, `assert_eq!`, a closure, an overflow), and its panic hook prints
each place. Every step is compared with what `cargo build` of the same source prints, and says
how many functions its build may compile.
"""
from includes import follow

MAIN = "src/main.rs"
MAIN_TEXT = """#[inline(never)]
fn above(x: u64) -> u64 {
    x + 1
}

#[track_caller]
fn checked(value: Option<u64>) -> u64 {
    value.expect("checked")
}

#[inline(never)]
fn panics(which: u32, n: u64) -> u64 {
    let v = vec![n];
    match which {
        0 => panic!("explicit {n}"),
        1 => None::<u64>.unwrap(),
        2 => v[n as usize + 5],
        3 => checked(None),
        4 => {
            assert_eq!(n, 0, "assert");
            n
        }
        5 => (|| -> u64 { panic!("in a closure") })(),
        _ => n + u64::MAX,
    }
}

fn main() {
    std::panic::set_hook(Box::new(|info| {
        let at = info.location().unwrap();
        println!("{}:{}:{}", at.file(), at.line(), at.column());
    }));
    let n = above(std::env::args().count() as u64) - 1;
    for which in 0..7 {
        let _ = std::panic::catch_unwind(|| panics(which, n));
    }
    println!("done");
}
"""
PANICS = "#[inline(never)]\nfn panics("
STEPS = [
    ("nothing edited", [], False, "compiled 0"),
    ("lines added above the function", [(MAIN, PANICS, "// Moved down.\n\n\n" + PANICS)], False, "compiled 0"),
    ("a line added to the function above it", [(MAIN, "    x + 1\n", "    let y = x + 1;\n    y\n")], False, "compiled 1"),
    ("lines added at the top of the file", [(MAIN, "#[inline(never)]\nfn above", "// One.\n// Two.\n\n#[inline(never)]\nfn above")], False, "compiled 0"),
    ("a line taken out above it", [(MAIN, "    let y = x + 1;\n    y\n", "    x + 1\n")], False, "compiled 1"),
    # With the closure written in it, which is a function of its own.
    ("the function itself edited", [(MAIN, '"explicit {n}"', '"explicitly {n}"')], False, "compiled 2"),
]

if __name__ == "__main__":
    follow([(MAIN, MAIN_TEXT)], STEPS)
