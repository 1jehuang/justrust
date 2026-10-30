//! tests/ui/let-else/let-else-drop-order.rs: when the pattern of a `let ... else` does not
//! match, the value is dropped before the `else` block runs, and so is a temporary the `let`
//! would have kept to the end of the block.

use std::cell::RefCell;

struct Noisy<'a>(u8, &'a RefCell<Vec<u8>>);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

enum Maybe<'a> {
    Some(Noisy<'a>, Noisy<'a>),
    None(Noisy<'a>),
}

#[inline(never)]
fn mismatch(log: &RefCell<Vec<u8>>) {
    let Maybe::Some(_, _kept) = Maybe::None(Noisy(1, log)) else {
        log.borrow_mut().push(2);
        return;
    };
}

#[inline(never)]
fn mismatch_wild(log: &RefCell<Vec<u8>>) {
    let Maybe::Some(_, _) = Maybe::None(Noisy(3, log)) else {
        log.borrow_mut().push(4);
        return;
    };
}

#[inline(never)]
fn make<'a>(log: &'a RefCell<Vec<u8>>, id: u8) -> (u8, Maybe<'a>) {
    (id, Maybe::None(Noisy(id, log)))
}

#[inline(never)]
fn mismatch_borrowed(log: &RefCell<Vec<u8>>) {
    let _first = Noisy(7, log);
    let &Maybe::Some(_, ref _kept) = &make(log, 5).1 else {
        log.borrow_mut().push(6);
        return;
    };
}

pub fn run() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    mismatch(&log);
    mismatch_wild(&log);
    mismatch_borrowed(&log);
    log.into_inner()
}
