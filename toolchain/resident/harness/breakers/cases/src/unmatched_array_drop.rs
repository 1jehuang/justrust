//! Found by the fuzzer (`fuzz/found/differs-checksum-L2-463-c8.rs`): an array that an `if let`
//! would move an element out of, where the pattern does not match. Nothing was moved out
//! then, so the array is dropped front to back like any other. ra-jit took the two ways out
//! of the pattern through one block, lost track of which it was on, and dropped the array as
//! one that has lost elements: from the back.

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

fn if_let(log: &Log, third: i32) {
    if let [_, second, Loud(-9..7, _), Loud(last, _)] =
        black_box([Loud(1, log), Loud(2, log), Loud(third, log), Loud(4, log)])
    {
        say(log, &format!("matched {} {last}", second.0));
    } else {
        say(log, "else");
    }
}

fn if_let_without_else(log: &Log, third: i32) {
    if let [first, .., Loud(3, _), _] = [Loud(1, log), Loud(2, log), Loud(third, log), Loud(4, log)] {
        say(log, &format!("matched {}", first.0));
    }
}

fn while_let(log: &Log, third: i32) {
    let mut rounds = 0;
    while let [.., Loud(3, _), last] = [Loud(rounds, log), Loud(third + rounds, log), Loud(4, log)] {
        say(log, &format!("matched {}", last.0));
        rounds += 1;
    }
}

fn matched(log: &Log, third: i32) {
    match [Loud(1, log), Loud(2, log), Loud(third, log), Loud(4, log)] {
        [_, second, Loud(3, _), _] => say(log, &format!("matched {}", second.0)),
        [first, ..] => say(log, &format!("other {}", first.0)),
    }
}

pub fn run() -> Vec<String> {
    let log = RefCell::new(Vec::new());
    for (name, case) in [
        ("if_let", if_let as fn(&Log, i32)),
        ("if_let_without_else", if_let_without_else),
        ("while_let", while_let),
        ("matched", matched),
    ] {
        for value in [black_box(3), black_box(30)] {
            say(&log, &format!("{name} {value}"));
            case(&log, value);
        }
    }
    log.into_inner()
}
