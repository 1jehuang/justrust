//! A move out of a field sets a bit in the drop flag of its local, by reading the flag and
//! writing it back, and an assignment to the field clears it the same way. Where every drop
//! of the local was decided when the body was generated nothing else reads the flag, and the
//! reads that only write it back go with its writes. Where a drop or a landing pad still
//! looks at a bit, on some way only, the flag and each change to it have to stay: what was
//! moved out is not dropped again, and what was left or put back is, once and in order.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(it: String) {
    LOG.with(|log| log.borrow_mut().push(it));
}

struct Noisy(String);

impl Noisy {
    fn of(name: &str) -> Noisy {
        Noisy(name.to_owned())
    }
}

impl Drop for Noisy {
    fn drop(&mut self) {
        log(format!("drop {}", self.0));
    }
}

struct Three {
    a: Noisy,
    b: Noisy,
    c: Noisy,
}

fn three(name: &str) -> Three {
    Three {
        a: Noisy(format!("{name}.a")),
        b: Noisy(format!("{name}.b")),
        c: Noisy(format!("{name}.c")),
    }
}

fn take(it: Noisy) {
    log(format!("took {}", it.0));
}

fn check(fail: bool) {
    if fail {
        panic!("asked to");
    }
}

/// Every field is moved out on the one way there is: nothing reads the flag.
#[inline(never)]
fn all_fields(it: Three, fail: bool) {
    take(it.a);
    take(it.b);
    take(it.c);
    check(fail);
    log("end".to_owned());
}

/// Fields are moved out one at a time with a call that may unwind after each.
#[inline(never)]
fn one_at_a_time(it: Three, fail_at: u32) {
    check(fail_at == 0);
    take(it.a);
    check(fail_at == 1);
    take(it.c);
    check(fail_at == 2);
    log("end".to_owned());
}

/// A field is moved out on one way only, so the drop where the ways meet reads its bit.
#[inline(never)]
fn on_one_way(it: Three, first: bool, second: bool, fail: bool) {
    if first {
        take(it.a);
    }
    if second {
        take(it.c);
    } else {
        take(it.b);
    }
    check(fail);
    log("end".to_owned());
}

/// A field is moved out and put back, time after time.
#[inline(never)]
fn put_back(mut it: Three, rounds: u32, fail_at: u32, leave_out: bool) {
    for round in 0..rounds {
        take(it.b);
        check(round == fail_at);
        if leave_out && round + 1 == rounds {
            break;
        }
        it.b = Noisy(format!("b{round}"));
        check(round + 100 == fail_at);
    }
    log("end".to_owned());
}

/// One local whose flag nothing reads beside one whose flag a landing pad does.
#[inline(never)]
fn side_by_side(spent: Three, kept: Three, move_kept: bool, fail: bool) {
    take(spent.c);
    take(spent.a);
    take(spent.b);
    if move_kept {
        take(kept.b);
    }
    check(fail);
    let (first, last) = (kept.a, kept.c);
    check(first.0.len() + last.0.len() == 0);
    log("end".to_owned());
}

/// Moved out of a tuple in a struct in an enum's payload.
#[inline(never)]
fn nested(it: (Three, Option<(Noisy, Noisy)>), deep: bool, fail: bool) {
    let (outer, inner) = it;
    take(outer.b);
    if let Some((first, second)) = inner {
        if deep {
            take(first);
        } else {
            take(second);
        }
    }
    check(fail);
    log("end".to_owned());
}

fn caught(run: impl FnOnce()) {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(()) => log("returned".to_owned()),
        Err(_) => log("panicked".to_owned()),
    }
}

pub fn run() -> Vec<String> {
    for fail in [false, true] {
        caught(|| all_fields(three("all"), fail));
        for first in [false, true] {
            for second in [false, true] {
                caught(|| on_one_way(three("way"), first, second, fail));
            }
        }
        for move_kept in [false, true] {
            caught(|| side_by_side(three("spent"), three("kept"), move_kept, fail));
        }
        for deep in [false, true] {
            caught(|| nested((three("outer"), Some((Noisy::of("i1"), Noisy::of("i2")))), deep, fail));
            caught(|| nested((three("outer"), None), deep, fail));
        }
    }
    for fail_at in 0..4 {
        caught(|| one_at_a_time(three("one"), fail_at));
    }
    for (rounds, fail_at) in [(0, 9), (1, 9), (3, 9), (3, 0), (3, 2), (3, 100), (3, 102)] {
        for leave_out in [false, true] {
            caught(|| put_back(three("back"), rounds, fail_at, leave_out));
        }
    }
    LOG.with(|log| log.take())
}
