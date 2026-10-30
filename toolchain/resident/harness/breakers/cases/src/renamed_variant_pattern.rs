//! delta's `format_and_paint_line_numbers`: `(true, Some(Right))` where `Right` is a variant
//! re-exported under another name, `pub use Index::Plus as Right`, by a module that itself
//! names the enum through a private import. A variant is as visible as its enum is declared,
//! but ra-jit gave it the visibility of the import the enum was reached by, so the re-export
//! was private, `Right` resolved to nothing elsewhere, and the pattern became a binding that
//! matched every side.

mod minusplus {
    #[derive(Clone, Copy, PartialEq, Debug)]
    pub enum Index {
        Minus,
        Plus,
    }
    pub use Index::*;
}

mod sides {
    use super::minusplus::*;
    pub use super::minusplus::Index as Side;
    pub use Index::Minus as Left;
    pub use Index::Plus as Right;

    pub fn first() -> Side {
        Minus
    }
}

mod numbers {
    use super::sides::{Left, Right, Side};

    #[inline(never)]
    pub fn emitted(both: bool, side: Option<Side>) -> (bool, bool) {
        match (both, side) {
            (false, _) => (true, true),
            (true, Some(Left)) => (true, false),
            (true, Some(Right)) => (false, true),
            (true, None) => (false, false),
        }
    }

    #[inline(never)]
    pub fn named(side: Side) -> &'static str {
        match side {
            Left => "left",
            Right => "right",
        }
    }

    pub fn all() -> [Side; 2] {
        [Left, Right]
    }
}

pub fn run() -> String {
    let [left, right] = numbers::all();
    let all = [
        numbers::emitted(false, None),
        numbers::emitted(true, Some(left)),
        numbers::emitted(true, Some(right)),
    ];
    format!("{all:?} {} {} {:?}", numbers::named(left), numbers::named(right), sides::first())
}
