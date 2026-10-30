//! Found by the fuzzer (`fuzz/found/differs-checksum-L7-50-c6.rs`): when the temporaries of
//! what an `if let` matches on are dropped. Before the 2024 edition they live as long as those
//! of the expression the `if` is in: to the end of the statement, or past the locals of the
//! block when the `if` is its tail. Only what the pattern binds ends with the branch. ra-jit
//! dropped them where the branch ended.

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
fn odd(loud: &Loud) -> Option<u32> {
    (loud.0 % 2 == 1).then_some(loud.0)
}

#[inline(never)]
fn both(a: u32, _b: &Loud) -> u32 {
    a
}

fn tail(n: u32) {
    let _kept = Loud(1);
    if let Some(found) = odd(&Loud(n)) {
        note(found + 100);
    } else {
        note(200);
    }
}

fn tail_of_block(n: u32) {
    {
        let _kept = Loud(2);
        if let _bound = &Loud(n) {
            note(300);
        }
    };
    note(400);
}

fn operand(n: u32) {
    let got = both(if let Some(found) = odd(&Loud(n)) { found } else { 0 }, &Loud(3));
    note(got + 500);
}

fn statement(n: u32) {
    let _kept = Loud(4);
    if let Some(found) = odd(&Loud(n)) {
        note(found + 600);
    }
    note(700);
}

fn owned(n: u32) -> u32 {
    let _kept = Loud(5);
    if let (Loud(found @ 0..=50), _other) = (Loud(n), Loud(n + 1)) { found } else { 0 }
}

fn repeated(n: u32) {
    let _kept = Loud(6);
    let mut left = n;
    while let Some(found) = odd(&Loud(left)) {
        note(found + 800);
        left -= 1;
    }
}

pub fn run() -> Vec<u32> {
    for n in [11, 12] {
        tail(n);
        tail_of_block(n);
        operand(n);
        statement(n);
        note(owned(n));
        note(owned(n + 60));
        repeated(n);
    }
    EVENTS.with(|it| it.take())
}
