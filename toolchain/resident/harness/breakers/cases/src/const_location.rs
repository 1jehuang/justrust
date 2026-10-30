//! A constant can take its value from where it is written: `line!()`, `column!()` and
//! `file!()` are the place of the call in it, as they are in a function.

const LINE: u32 = line!();
const PLACE: (u32, u32) = (line!() + 1, column!());
const FILE: &str = file!();

struct Site {
    file: &'static str,
    line: u32,
}

fn inner() -> (u32, &'static str) {
    const SITE: Site = Site { file: file!(), line: line!() };
    (SITE.line, SITE.file)
}

pub fn run() -> (u32, (u32, u32), &'static str, (u32, &'static str)) {
    (LINE, PLACE, FILE, inner())
}
