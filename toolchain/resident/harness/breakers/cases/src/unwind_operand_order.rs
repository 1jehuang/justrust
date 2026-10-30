//! Found by the fuzzer (`fuzz/found/differs-checksum-s2-23-c24.rs`): the order of drops when
//! an operand of an array, a tuple or a call panics. The operands already made belong to the
//! expression they are operands of, and a `&temporary` made on the way lives to the end of
//! the statement, so rustc drops the operands first. ra-jit dropped the first operand last,
//! after the temporaries that were made later than it.
//!
//! Found later (`fuzz/found/differs-checksum-L8-231-c11.rs`): where the expression ends its
//! own temporaries, as the tail of a block does from the 2024 edition and a closure's body
//! without braces does, operands and temporaries are in one scope and are dropped in the
//! reverse of the order they were made. `breakers2021::unwind_operand_tail` has the tails
//! before 2024.

use std::cell::RefCell;
use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

thread_local! {
    static DROPPED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

struct Loud(u32);

impl Drop for Loud {
    fn drop(&mut self) {
        DROPPED.with(|it| it.borrow_mut().push(self.0));
    }
}

#[inline(never)]
fn none() -> Option<Loud> {
    None
}

#[inline(never)]
fn sum(a: &Loud, b: &Loud) -> Loud {
    Loud(a.0 + b.0)
}

fn take(_a: Loud, _b: Loud, _c: Loud, _d: Loud) {}

fn take_one(items: (Loud, Loud, Loud)) -> Loud {
    items.0
}

struct Fields {
    _a: Loud,
    _b: Loud,
    _c: Loud,
}

/// The order in which `f` dropped what it made.
fn order<T>(f: impl FnOnce() -> T) -> Vec<u32> {
    let _ = catch_unwind(AssertUnwindSafe(f));
    DROPPED.with(|it| it.take())
}

pub fn run() -> Vec<Vec<u32>> {
    vec![
        order(|| {
            let _items = [Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap(), Loud(4)];
        }),
        order(|| {
            let _items = (Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap(), Loud(4));
        }),
        order(|| take(Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap())),
        order(|| {
            let _items = Fields { _a: Loud(1), _b: sum(&Loud(20), &Loud(30)), _c: none().unwrap() };
        }),
        order(|| {
            let _items = [Loud(1), sum(&Loud(20), &Loud(30)), Loud(3)];
        }),
        order(|| (Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap())),
        order(|| -> [Loud; 4] { [Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap()] }),
        order(|| -> Fields { Fields { _a: Loud(1), _b: sum(&Loud(20), &Loud(30)), _c: none().unwrap() } }),
        order(|| -> Loud { sum(&Loud(1), &take_one((Loud(2), sum(&Loud(20), &Loud(30)), none().unwrap()))) }),
        order(|| -> (Loud, Box<Loud>, u32) { (Loud(1), Box::new(sum(&Loud(20), &Loud(30))), 1 / black_box(0)) }),
    ]
}
