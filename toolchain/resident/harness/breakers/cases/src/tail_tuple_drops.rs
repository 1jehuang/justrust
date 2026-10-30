//! tests/ui/drop/drop-order-comparisons.rs: the temporaries of a block's tail expression.
//! This crate is edition 2024, where they are dropped before the block's value is used.

use std::cell::RefCell;

struct Noisy<'a>(u8, &'a RefCell<Vec<u8>>);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

impl Noisy<'_> {
    fn touch(&self) -> bool {
        true
    }
}

fn mark(n: u8, log: &RefCell<Vec<u8>>) {
    log.borrow_mut().push(n);
}

#[inline(never)]
fn tail_tuple(log: &RefCell<Vec<u8>>) {
    _ = ({ (Noisy(4, log), Noisy(2, log).touch(), Noisy(5, log), Noisy(1, log).touch()) }, mark(3, log), Noisy(6, log));
}

#[inline(never)]
fn if_let_then(log: &RefCell<Vec<u8>>) {
    _ = (
        if let Some(_) = Some(&Noisy(12, log)) {
            mark(11, log);
        },
        mark(13, log),
        Noisy(14, log),
    );
}

pub fn run() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    tail_tuple(&log);
    if_let_then(&log);
    log.into_inner()
}
