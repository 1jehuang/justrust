//! Found by the fuzzer (`fuzz/found/differs-checksum-L6-176-c3.rs`): when the temporaries of
//! what a `for` iterates over are dropped. rustc drops them where the loop ends. Before the
//! 2024 edition that shows when the loop is the tail of a block: the temporaries of any other
//! tail outlive the locals of the block, and those of a `for` do not. ra-jit kept them like
//! any other tail's.

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
fn items(loud: &Loud) -> [u32; 2] {
    [loud.0 + 1, loud.0 + 2]
}

fn function_tail(n: u32) {
    let _kept = Loud(1);
    for item in items(&Loud(n)) {
        note(item);
    }
}

fn block_tail(n: u32) {
    let unit = {
        let _kept = Loud(2);
        for item in items(&Loud(n)) {
            note(item);
        }
    };
    note(100);
    unit
}

fn left_early(n: u32) {
    let _kept = Loud(3);
    'outer: for item in items(&Loud(n)) {
        for inner in items(&Loud(item * 10)) {
            if inner % 2 == 0 {
                continue 'outer;
            }
            note(inner);
        }
        break;
    }
}

fn borrowed(n: u32) {
    let _kept = Loud(4);
    for loud in [Loud(n), Loud(n + 1)].iter() {
        note(loud.0 + 5);
    }
}

fn other_tail(n: u32) -> u32 {
    let _kept = Loud(5);
    items(&Loud(n))[0]
}

pub fn run() -> Vec<u32> {
    function_tail(10);
    block_tail(20);
    left_early(30);
    borrowed(40);
    note(other_tail(50));
    EVENTS.with(|it| it.take())
}
