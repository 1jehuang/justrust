//! Found by the fuzzer (`fuzz/found/left-mir-unresolvedupvar-L2-1-c27.rs`): a closure that
//! matches on a field of a variable with patterns that bind nothing, and uses another field
//! of it. What is matched on is not read, so the closure does not capture it, and ra-jit
//! found no capture for the place: the closure's caller was left to rustc.

use std::cell::RefCell;

thread_local! {
    static EVENTS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

fn note(event: u32) {
    EVENTS.with(|it| it.borrow_mut().push(event));
}

struct Loud(u32);

impl Drop for Loud {
    fn drop(&mut self) {
        note(self.0);
    }
}

#[inline(never)]
fn guarded(x: (char, (char, i8))) -> bool {
    let test = || {
        (match x.1 {
            (_, _) if x.0 == 'q' => 'a',
            _ => x.0,
        })
        .is_whitespace()
    };
    test()
}

#[inline(never)]
fn wildcard(x: (u32, (char, i8))) -> u32 {
    let pick = || match x.1 {
        _ => x.0,
    };
    pick()
}

#[inline(never)]
fn nothing_captured(x: (u32, (char, i8))) -> u32 {
    let five = || match x.1 {
        _ => 5,
    };
    five() + x.0
}

/// The closure owns `x.0` only: `x.1` stays here and is dropped when this returns.
#[inline(never)]
fn moved() {
    let x = (Loud(1), (Loud(2), Loud(3)));
    let take = move || {
        let (_, _) = x.1;
        let _ = x.1;
        note(10);
        x.0
    };
    let first = take();
    note(20);
    drop(first);
    note(30);
}

pub fn run() -> (bool, u32, u32, Vec<u32>) {
    moved();
    (
        guarded((' ', ('b', 1))),
        wildcard((7, ('b', 1))),
        nothing_captured((8, ('b', 1))),
        EVENTS.with(|it| it.take()),
    )
}
