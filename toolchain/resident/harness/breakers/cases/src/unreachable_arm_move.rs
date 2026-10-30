//! Found by the fuzzer (`fuzz/found/differs-checksum-L7-230-c24.rs`): a `match` on an array
//! with an arm that would move elements out and that no value reaches, because an arm
//! before it takes every value it matches (rustc warns of an unreachable pattern). rustc
//! makes one tree of tests of all the arms, and when no edge of it leads to such an arm it
//! knows the array is whole where it is dropped, and drops it front to back. ra-jit tried
//! the arms one after the other, saw a way through the move, and dropped the array as one
//! that has lost elements: from the back. Which arms the tree leads to is not which arms a
//! value can reach: the cases named `still_*` have an arm that nothing reaches and that
//! rustc's tree does lead to.

#![allow(unreachable_patterns)]

use std::cell::RefCell;
use std::hint::black_box;

type Log = RefCell<Vec<String>>;

enum Loud<'a> {
    Int(i32, &'a Log),
    Char(char, &'a Log),
}

impl Drop for Loud<'_> {
    fn drop(&mut self) {
        match self {
            Loud::Int(n, log) => log.borrow_mut().push(format!("drop {n}")),
            Loud::Char(c, log) => log.borrow_mut().push(format!("drop {c}")),
        }
    }
}

fn items(log: &Log, last: char) -> [Loud<'_>; 3] {
    black_box([Loud::Int(1, log), Loud::Char('q', log), Loud::Char(last, log)])
}

fn starting(log: &Log, first: char) -> [Loud<'_>; 3] {
    black_box([Loud::Char(first, log), Loud::Char('q', log), Loud::Int(2, log)])
}

fn covered_by_binding(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [.., Loud::Char(_, _)] => 1,
        [Loud::Char(..), _middle @ .., Loud::Char('Z', _)] => 2,
        _ => 3,
    }
}

fn covered_with_guard(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [.., Loud::Char(_, _)] => 1,
        [_first, .., Loud::Char('Z', _)] if black_box(last) == 'Z' => 2,
        _ => 3,
    }
}

fn covered_by_alternative(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [.., Loud::Char('a', _) | Loud::Char(_, _)] => 1,
        [_middle @ .., Loud::Char(_, _)] => 2,
        _ => 3,
    }
}

fn covered_inside(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [Loud::Int(_, _), .., Loud::Char(_, _)] => 1,
        [Loud::Int(1, _), _middle @ .., Loud::Char(_, _)] => 2,
        _ => 3,
    }
}

fn covered_number(log: &Log, last: char) -> u8 {
    match (items(log, last), black_box(1u8)) {
        (_, 1) => 1,
        ([_middle @ .., _], 1) => 2,
        _ => 3,
    }
}

fn still_behind_a_guard(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [.., Loud::Char(_, _)] => 1,
        _ if black_box(false) => 2,
        [_middle @ .., Loud::Char(_, _)] => 3,
        _ => 4,
    }
}

fn still_after_every_variant(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [.., Loud::Char(_, _)] => 1,
        [.., Loud::Int(_, _)] => 2,
        [_middle @ .., _] => 3,
    }
}

fn still_behind_a_range(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [.., Loud::Char('a'..='z', _)] => 1,
        [_middle @ .., Loud::Char('a'..='c', _)] => 2,
        _ => 3,
    }
}

/// Finding 21 (`fuzz/found/differs-checksum-L12-18-c0.rs`): an arm that is only an
/// or-pattern is expanded into its alternatives before the test of the arm in front of it is
/// made, and the alternatives are sorted by that test like any other arm. So the range test
/// of the first arm sends the third arm with it, and the third is never reached. ra-jit
/// expanded the or-pattern only when its arm came first, and the third arm was tested on
/// its own after the second.
fn covered_past_an_alternative(log: &Log, first: char) -> u8 {
    match starting(log, first) {
        [Loud::Char('A'..='Z', _), ..] => 1,
        [Loud::Char('b', _) | Loud::Int(..3, _), ..] => 2,
        [Loud::Char('A'..='Z', _), _middle @ .., _] => 3,
        _ => 4,
    }
}

/// The same with the or-pattern first: its alternatives make the tests, and the range test
/// that the first alternative fails sends the other two arms along, so the third is still
/// never reached.
fn covered_after_an_alternative(log: &Log, first: char) -> u8 {
    match starting(log, first) {
        [Loud::Char('b', _) | Loud::Int(..3, _), ..] => 1,
        [Loud::Char('A'..='Z', _), ..] => 2,
        [Loud::Char('A'..='Z', _), _middle @ .., _] => 3,
        _ => 4,
    }
}

/// An arm with two or-patterns is expanded too, but the arms after it wait until it is
/// done, so here the third arm is reached.
fn still_past_two_alternatives(log: &Log, first: char) -> u8 {
    match starting(log, first) {
        [Loud::Char('A'..='Z', _), ..] => 1,
        [Loud::Char('b', _) | Loud::Int(..3, _), Loud::Char('x', _) | Loud::Int(1, _), _] => 2,
        [Loud::Char('A'..='Z', _), _middle @ .., _] => 3,
        _ => 4,
    }
}

fn reached_through_a_guard(log: &Log, last: char) -> u8 {
    match items(log, last) {
        [.., Loud::Char(_, _)] if black_box(last) == 'a' => 1,
        [_middle @ .., Loud::Char(_, _)] => 2,
        _ => 3,
    }
}

pub fn run() -> Vec<String> {
    let log = RefCell::new(Vec::new());
    let cases = [
        covered_by_binding as fn(&Log, char) -> u8,
        covered_with_guard,
        covered_by_alternative,
        covered_inside,
        covered_number,
        still_behind_a_guard,
        still_after_every_variant,
        still_behind_a_range,
        reached_through_a_guard,
        covered_past_an_alternative,
        covered_after_an_alternative,
        still_past_two_alternatives,
    ];
    for last in ['a', 'Z', 'b'] {
        for case in cases {
            let arm = case(&log, last);
            log.borrow_mut().push(format!("arm {arm}"));
        }
    }
    log.into_inner()
}
