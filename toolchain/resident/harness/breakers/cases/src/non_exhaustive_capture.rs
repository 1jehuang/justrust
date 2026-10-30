//! tests/ui/closures/2229_closure_analysis/match/partial-move-drop-order.rs: a closure that
//! matches on a `#[non_exhaustive]` enum of one variant captures the whole enum, in the
//! crate that defines the enum as in any other. It captured the one field the pattern
//! binds, so the enum was still the function's to drop when it was assigned to.

use std::cell::RefCell;

struct Noisy<'a>(u8, &'a RefCell<Vec<u8>>);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

enum Sealed<'a> {
    One(i32, Noisy<'a>),
}

#[non_exhaustive]
enum Open<'a> {
    One(i32, Noisy<'a>),
}

#[allow(unused)]
#[inline(never)]
fn sealed(log: &RefCell<Vec<u8>>) {
    let mut thing = Sealed::One(0, Noisy(1, log));
    let closure = move || match thing {
        Sealed::One(x, _) => {}
    };
    thing = Sealed::One(1, Noisy(3, log));
    log.borrow_mut().push(2);
}

#[allow(unused)]
#[inline(never)]
fn open(log: &RefCell<Vec<u8>>) {
    let mut thing = Open::One(0, Noisy(12, log));
    let closure = move || match thing {
        Open::One(x, _) => {}
    };
    thing = Open::One(1, Noisy(13, log));
    log.borrow_mut().push(11);
}

pub fn run() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    sealed(&log);
    open(&log);
    log.into_inner()
}
