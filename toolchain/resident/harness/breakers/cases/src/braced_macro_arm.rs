//! tests/ui/parser/macro/statement-boundaries.rs: a macro called with braces ends a statement,
//! and does not end what a match arm gives, where `|` after it is an operator. ra-jit ended the
//! arm there and read `| Mark` as the pattern of an arm that was not written.

macro_rules! unit {
    () => {
        ()
    };
}

#[derive(Copy, Clone)]
struct Mark;

impl std::ops::BitOr<Mark> for () {
    type Output = u32;
    fn bitor(self, _: Mark) -> u32 {
        7
    }
}

#[inline(never)]
fn arm(mark: Mark, taken: bool) -> u32 {
    match mark {
        Mark if taken => unit! {} | Mark,
        Mark => 1,
    }
}

pub fn run() -> (u32, u32) {
    (arm(Mark, true), arm(Mark, false))
}
