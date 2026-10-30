//! Found by the fuzzer (`fuzz/found/differs-checksum-S1-2-c7.rs`): the order of drops when an
//! operand panics in an aggregate or a call that is the tail of a block with locals of its
//! own. The operands made so far are dropped before the block's locals, in every edition;
//! only the temporaries of the tail outlive the block before 2024. ra-jit handed the operands
//! to the enclosing scope along with the temporaries and dropped the locals first.

use std::cell::RefCell;
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
fn trip(armed: bool, value: Loud) -> Loud {
    if armed {
        std::panic::panic_any(1u32);
    }
    value
}

#[inline(never)]
fn sum(a: &Loud, b: &Loud) -> Loud {
    Loud(a.0 + b.0)
}

fn take(_a: Loud, _b: Loud, _c: Loud) {}

/// The order in which `f` dropped what it made.
fn order(f: impl FnOnce()) -> Vec<u32> {
    let _ = catch_unwind(AssertUnwindSafe(f));
    DROPPED.with(|it| it.take())
}

fn orders(armed: bool) -> Vec<Vec<u32>> {
    vec![
        order(|| {
            let _v = {
                let _local = Loud(1);
                (Loud(2), trip(armed, Loud(3)))
            };
        }),
        order(|| {
            let _v = {
                let _local = Loud(1);
                (Loud(2), sum(&Loud(20), &Loud(30)), trip(armed, Loud(3)))
            };
        }),
        order(|| {
            let _v = {
                let _local = Loud(1);
                let _other = Loud(4);
                [Loud(2), (Loud(5), trip(armed, Loud(3))).0]
            };
        }),
        order(|| {
            let _local = Loud(1);
            take(Loud(2), sum(&Loud(20), &Loud(30)), trip(armed, Loud(3)))
        }),
        order(|| {
            let _v = {
                let _local = Loud(1);
                {
                    let _inner = Loud(4);
                    (Loud(2), trip(armed, Loud(3)))
                }
            };
        }),
    ]
}

pub fn run() -> Vec<Vec<u32>> {
    [false, true].into_iter().flat_map(orders).collect()
}
