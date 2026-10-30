//! library/coretests panic/location.rs: `Location::caller()` in a constant is where the
//! constant calls it. ra-jit evaluated it to where `caller` asks, inside `core`, under a file
//! name rustc does not use. What rustc names the file is not known where a constant is
//! evaluated, so a function with such a constant is left to rustc.

use std::panic::Location;

#[inline(never)]
fn asked() -> (u32, u32, bool) {
    const CALLER: &Location<'static> = Location::caller();
    const LINE: u32 = CALLER.line();
    const COLUMN: u32 = CALLER.column();
    (LINE, COLUMN, CALLER.file() == file!())
}

pub fn run() -> (u32, u32, bool) {
    asked()
}
