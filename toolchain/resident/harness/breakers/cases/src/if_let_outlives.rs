//! tests/ui/drop/drop-order-comparisons.rs, revision e2021: see `breakers2021::if_let_outlives`.

use std::cell::RefCell;

pub fn run() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    breakers2021::if_let_outlives(&log);
    log.into_inner()
}
