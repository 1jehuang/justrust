//! Found by the fuzzer (`fuzz/found/differs-checksum-L11-165-c0.rs`): the order of drops when
//! the body of a loop panics and the loop is inside another loop. What the inner loop's body
//! holds is dropped before what the loop iterates over. ra-jit read the order of the scopes off
//! the order of the blocks, which inside a loop says nothing: the exit of the inner loop and
//! its body each lead to the other, and what was iterated over was dropped first.

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
fn trip(armed: bool, value: &Loud) -> u32 {
    if armed && value.0 % 10 == 2 {
        std::panic::panic_any(1u32);
    }
    value.0
}

/// The order in which `f` dropped what it made.
fn order(f: impl FnOnce()) -> Vec<u32> {
    let _ = catch_unwind(AssertUnwindSafe(f));
    DROPPED.with(|it| it.take())
}

fn orders(armed: bool) -> Vec<Vec<u32>> {
    vec![
        order(|| {
            for round in 0..2 {
                for item in vec![Loud(round * 10 + 1), Loud(round * 10 + 2), Loud(round * 10 + 3)] {
                    trip(armed, &item);
                }
            }
        }),
        order(|| {
            for round in 0..2 {
                for (_, item) in (0..3).zip(vec![Loud(round * 10 + 1), Loud(round * 10 + 2), Loud(3)]) {
                    let held = Loud(round + 40);
                    trip(armed, &item);
                    drop(held);
                }
            }
        }),
        order(|| {
            let mut round = 0;
            while round < 2 {
                let outer = Loud(50 + round);
                let mut items = vec![Loud(1), Loud(2), Loud(3)].into_iter();
                while let Some(item) = items.next() {
                    let inner = Loud(60 + round);
                    trip(armed, &item);
                    trip(armed, &inner);
                }
                trip(false, &outer);
                round += 1;
            }
        }),
        order(|| {
            for outer in vec![Loud(71), Loud(73)] {
                loop {
                    let held = Loud(80);
                    for item in [Loud(1), Loud(2)] {
                        trip(armed, &item);
                    }
                    trip(false, &held);
                    break;
                }
                trip(false, &outer);
            }
        }),
    ]
}

pub fn run() -> Vec<Vec<u32>> {
    [false, true].into_iter().flat_map(orders).collect()
}
