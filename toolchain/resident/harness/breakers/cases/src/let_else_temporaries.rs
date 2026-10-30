//! Found by the fuzzer (`fuzz/found/differs-checksum-L2-372-c9.rs`, `-L2-354-c5.rs`,
//! `-L2-431-c24.rs`): a `let` with an `else` whose pattern does not match. The temporaries of
//! the initializer, the value that was matched among them, are dropped before the `else`
//! runs, and an array none of whose elements were moved out is dropped front to back. ra-jit
//! dropped them after the `else`, and the array from the back.

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

fn array(log: &Log, last: i32) {
    for round in 0..2 {
        let [Loud(..), rest @ .., Loud(7, _)] = black_box([Loud(1, log), Loud(2, log), Loud(last, log)])
        else {
            say(log, "else");
            break;
        };
        say(log, &format!("matched {} in round {round}", rest.len()));
    }
}

fn tuple(log: &Log, first: i32) {
    let (Loud(1, _), second) = (Loud(first, log), Loud(5, log)) else {
        return say(log, "else");
    };
    say(log, &format!("matched {}", second.0));
}

fn temporary(log: &Log, first: i32) {
    let Some(found) = Some(&Loud(first, log)).filter(|it| it.0 == 1).map(|it| it.0) else {
        return say(log, "else");
    };
    say(log, &format!("matched {found}"));
}

fn extended(log: &Log, first: i32) {
    let (Loud(1, _), second) = &(Loud(first, log), Loud(5, log)) else {
        return say(log, "else");
    };
    say(log, &format!("matched {}", second.0));
}

fn named(log: &Log, first: i32) {
    let held = (Loud(first, log), Loud(6, log));
    let (Loud(1, _), second) = held else {
        return say(log, "else");
    };
    say(log, &format!("matched {}", second.0));
}

fn in_loop(log: &Log, sum: i32) {
    'outer: loop {
        for at in 0..3 {
            let [first, Loud(1, _)] = [Loud(at, log), Loud(sum - at, log)] else {
                say(log, "else");
                if at == 2 {
                    break 'outer;
                }
                continue;
            };
            say(log, &format!("matched {}", first.0));
        }
    }
}

pub fn run() -> Vec<String> {
    let log = RefCell::new(Vec::new());
    for (name, case) in [
        ("array", array as fn(&Log, i32)),
        ("tuple", tuple),
        ("temporary", temporary),
        ("extended", extended),
        ("named", named),
        ("in_loop", in_loop),
    ] {
        for value in [black_box(1), black_box(2), black_box(7)] {
            say(&log, &format!("{name} {value}"));
            case(&log, value);
        }
    }
    log.into_inner()
}
