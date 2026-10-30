//! Found by the fuzzer (`fuzz/found/differs-checksum-L1-12-c3.rs`): when the temporaries of
//! the tail of an `if` branch are dropped. Before the 2024 edition the temporaries of a
//! block's tail live as long as those of the expression the block is in, but a branch of an
//! `if` ends them: rustc drops them where the branch ends. ra-jit kept them to the end of the
//! statement the `if` was in.

use std::cell::RefCell;

thread_local! {
    static EVENTS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
}

fn note(event: u32) {
    EVENTS.with(|it| it.borrow_mut().push(event));
}

#[derive(Clone)]
struct Loud(u32);

impl Drop for Loud {
    fn drop(&mut self) {
        note(self.0);
    }
}

#[inline(never)]
fn second<'a>(_a: &'a Loud, b: &'a Loud) -> &'a Loud {
    b
}

#[inline(never)]
fn seen(_value: &Loud) {
    note(0);
}

fn branch(first: bool) {
    seen(&if first { Loud(1) } else { second(&Loud(2), &Loud(3)).clone() });
    note(100);
}

fn nested(first: bool) {
    seen(&if first {
        second(&Loud(4), &Loud(5)).clone()
    } else if Box::new(first) != Box::new(false) {
        Loud(6)
    } else {
        second(&Loud(7), &Loud(8)).clone()
    });
    note(100);
}

pub fn run() -> Vec<u32> {
    branch(false);
    branch(true);
    nested(false);
    nested(true);
    EVENTS.with(|it| it.take())
}
