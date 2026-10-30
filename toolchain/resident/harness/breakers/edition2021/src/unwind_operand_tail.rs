//! Found by the fuzzer (`fuzz/found/differs-checksum-L8-231-c11.rs`): the order of drops when
//! an operand panics and the aggregate or call is the tail of a block. Before the 2024
//! edition the temporaries of a tail are those of the expression the block is in, so the
//! operands, which belong to the tail, are dropped before them, as in a statement. ra-jit
//! only did that for statements and dropped the first operand last.

use std::cell::RefCell;
use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

thread_local! {
    static DROPPED: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

pub struct Loud(u32);

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

pub struct Three(Loud, Box<Loud>, u32);

pub struct Fields {
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
        order(|| -> (Loud, Loud, Loud, Loud) { (Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap()) }),
        order(|| -> [Loud; 4] { [Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap()] }),
        order(|| -> Fields { Fields { _a: Loud(1), _b: sum(&Loud(20), &Loud(30)), _c: none().unwrap() } }),
        order(|| -> Three { Three(Loud(1), Box::new(sum(&Loud(20), &Loud(30))), 1 / black_box(0)) }),
        order(|| -> () { take(Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap()) }),
        order(|| -> Loud { sum(&Loud(1), &take_one((Loud(2), sum(&Loud(20), &Loud(30)), none().unwrap()))) }),
        order(|| (Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap())),
        order(|| take(Loud(1), sum(&Loud(20), &Loud(30)), Loud(3), none().unwrap())),
    ]
}
