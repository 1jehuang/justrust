//! Found by the fuzzer (`edits.py smoke`, the second program): an array that is one field of
//! a matched tuple, where one arm moves its elements out with `[x, m @ ..]` and a later one
//! takes the whole array. The arm that took all of it drops all of it, once. ra-jit went on to
//! drop the elements again on the way out, from the back, as of an array that had lost some.

use std::cell::RefCell;
use std::hint::black_box;

type Log = RefCell<Vec<String>>;

enum Loud<'a> {
    I(i32, Noisy<'a>),
    C(char, Noisy<'a>),
}

struct Noisy<'a>(u32, &'a Log);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(format!("drop {}", self.0));
    }
}

#[inline(never)]
fn items(log: &Log, k: u32, last: char) -> [Loud<'_>; 3] {
    black_box([
        Loud::I(k as i32 % 4, Noisy(k, log)),
        Loud::C('q', Noisy(k + 1, log)),
        if k % 3 == 0 { Loud::I(1, Noisy(k + 2, log)) } else { Loud::C(last, Noisy(k + 2, log)) },
    ])
}

#[inline(never)]
fn pair(log: &Log, k: u32, last: char, flag: bool) -> u8 {
    match (items(log, k, last), k % 3, flag) {
        ([Loud::C('b', _), ..], _, true) => 1,
        ([.., Loud::I(1, _)], 1, _) => 2,
        ([x, m @ ..], _, false) => 3,
        (all, _, _) if black_box(flag) => 4,
        _ => 0,
    }
}

#[inline(never)]
fn plain(log: &Log, k: u32, flag: bool) -> u8 {
    match (items(log, k, 'a'), flag) {
        ([x, m @ ..], false) => 1,
        (all, _) if black_box(flag) => 2,
        _ => 0,
    }
}

#[inline(never)]
fn bare(log: &Log, k: u32, flag: bool) -> u8 {
    match items(log, k, 'a') {
        [x, m @ ..] if black_box(!flag) => 1,
        all if black_box(flag) => 2,
        _ => 0,
    }
}

#[inline(never)]
fn unguarded(log: &Log, k: u32, flag: bool) -> u8 {
    match (items(log, k, 'a'), flag) {
        ([x, m @ ..], false) => 1,
        (all, _) => 2,
    }
}

pub fn run() -> Vec<String> {
    let log = RefCell::new(Vec::new());
    for k in 0..4u32 {
        for last in ['a', 'Z'] {
            for flag in [false, true] {
                let picked = pair(&log, k, last, flag);
                log.borrow_mut().push(format!("pair {k} {last} {flag}: {picked}"));
            }
        }
        for (name, case) in
            [("plain", plain as fn(&Log, u32, bool) -> u8), ("bare", bare), ("unguarded", unguarded)]
        {
            for flag in [false, true] {
                let picked = case(&log, k, flag);
                log.borrow_mut().push(format!("{name} {k} {flag}: {picked}"));
            }
        }
    }
    log.into_inner()
}
