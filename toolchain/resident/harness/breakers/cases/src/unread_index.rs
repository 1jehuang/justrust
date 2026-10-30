//! Found by the fuzzer (`fuzz/found/differs-panic-location-L2-154-c15.rs`): an element of an
//! array or slice that is matched on by a pattern that reads nothing. The index is still
//! checked, so `let _ = items[at]` panics when `at` is out of bounds. ra-jit made no code for
//! a place that is not read, and the program went on.

use std::hint::black_box;
use std::panic::{AssertUnwindSafe, catch_unwind};

fn wild_match(items: &[i8; 3], at: usize) -> u8 {
    match items[at] {
        _ => 1,
    }
}

fn wild_let(items: &[i8], at: usize) -> u8 {
    let _ = items[at];
    2
}

fn wild_tuple(items: &[(i8, i8); 3], at: usize) -> u8 {
    let (_, _) = items[at];
    3
}

fn wild_field(items: &[(i8, i8); 3], at: usize) -> u8 {
    let _ = items[at].1;
    4
}

fn wild_deref(items: &[Box<i8>], at: usize) -> u8 {
    let _ = *items[at];
    5
}

fn wild_nested(items: &[[i8; 2]; 2], at: usize) -> u8 {
    let _ = items[1][at];
    6
}

fn in_closure(items: [i8; 3], at: usize) -> u8 {
    let check = || {
        let _ = items[at];
        7
    };
    check()
}

/// Whether `f` ended, or the line of the message of its panic.
fn outcome(f: impl FnOnce() -> u8) -> Result<u8, String> {
    catch_unwind(AssertUnwindSafe(f)).map_err(|payload| match payload.downcast::<String>() {
        Ok(message) => *message,
        Err(_) => String::from("not a message"),
    })
}

pub fn run() -> Vec<Result<u8, String>> {
    let mut outcomes = Vec::new();
    for at in [black_box(1usize), black_box(7usize)] {
        outcomes.push(outcome(|| wild_match(&[1, 2, 3], at)));
        outcomes.push(outcome(|| wild_let(&[1, 2, 3], at)));
        outcomes.push(outcome(|| wild_tuple(&[(1, 2); 3], at)));
        outcomes.push(outcome(|| wild_field(&[(1, 2); 3], at)));
        outcomes.push(outcome(|| wild_deref(&[Box::new(1), Box::new(2)], at)));
        outcomes.push(outcome(|| wild_nested(&[[1, 2]; 2], at)));
        outcomes.push(outcome(|| in_closure([1, 2, 3], at)));
    }
    outcomes
}
