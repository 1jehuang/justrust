//! delta names the two panels of its side-by-side view by importing the variants of an enum
//! under other names, through a name that is itself in scope only by a glob import.

mod index {
    #[derive(Debug, Clone, Copy, PartialEq)]
    pub enum MinusPlusIndex {
        Minus,
        Plus,
    }
}

mod panels {
    use super::index::*;

    pub use super::index::MinusPlusIndex as PanelSide;
    pub use MinusPlusIndex::Minus as Left;
    pub use MinusPlusIndex::Plus as Right;

    pub fn width(side: PanelSide) -> usize {
        match side {
            Left => 40,
            Right => 39,
        }
    }
}

use panels::{Left, Right};

pub fn run() -> (usize, usize, bool, String) {
    let sides = [Left, Right];
    (panels::width(sides[0]), panels::width(panels::Right), Left == sides[1], format!("{:?}", Right))
}
