//! Found by the fuzzer (`fuzz/found/differs-checksum-L7-174-c9.rs`): an arm that binds the
//! middle of an array by value (`[_, middle @ .., _] if guard`) and whose guard is false. The
//! elements go back into the array for the arms that follow. ra-jit put them back and did
//! not note that they were there again, so the array was dropped without them: they were
//! never dropped at all.

use std::cell::RefCell;
use std::hint::black_box;

type Log = RefCell<Vec<String>>;

struct Loud<'a>(i32, &'a Log);

impl Drop for Loud<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(format!("drop {}", self.0));
    }
}

fn say(log: &Log, what: &str) {
    log.borrow_mut().push(what.to_owned());
}

fn middle_of_three(log: &Log, first: i32, on: bool) {
    match black_box([Loud(first, log), Loud(first + 1, log), Loud(first + 2, log)]) {
        [Loud(_, _), middle @ .., Loud(3..=40, _)] if on => {
            let [taken] = middle;
            say(log, &format!("took {}", taken.0));
        }
        _ => say(log, "other"),
    }
    say(log, "end");
}

fn rest_of_four(log: &Log, first: i32, on: bool) {
    match [Loud(first, log), Loud(first + 1, log), Loud(first + 2, log), Loud(first + 3, log)] {
        [head, rest @ ..] if black_box(on) => say(log, &format!("took {} {}", head.0, rest.len())),
        [.., last] if black_box(!on) => say(log, &format!("last {}", last.0)),
        _ => say(log, "other"),
    }
    say(log, "end");
}

fn in_a_tuple(log: &Log, first: i32, on: bool) {
    match (Loud(first, log), [Loud(first + 1, log), Loud(first + 2, log)]) {
        (_, [_, tail @ ..]) if black_box(on) => say(log, &format!("took {}", tail.len())),
        (kept, _) => say(log, &format!("kept {}", kept.0)),
    }
    say(log, "end");
}

pub fn run() -> Vec<String> {
    let log = RefCell::new(Vec::new());
    for on in [false, true] {
        middle_of_three(&log, 10, on);
        middle_of_three(&log, 100, on);
        rest_of_four(&log, 20, on);
        in_a_tuple(&log, 30, on);
    }
    log.into_inner()
}
