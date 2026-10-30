//! A `match` that moves the payload out of each variant in its own arm leaves a value with
//! nothing to drop where the arms meet, though no one field is moved out on every way. The
//! drop after the arms is then left out, and a call that unwinds past it has nothing to do
//! for the value. What is not moved out on some way still has to be dropped, once, in order.

use std::cell::RefCell;
use std::panic::{AssertUnwindSafe, catch_unwind};

thread_local! {
    static LOG: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

fn log(it: String) {
    LOG.with(|log| log.borrow_mut().push(it));
}

struct Noisy(&'static str);

impl Drop for Noisy {
    fn drop(&mut self) {
        log(format!("drop {}", self.0));
    }
}

enum Either {
    Left(Noisy),
    Right(Noisy, Noisy),
    Plain(u32),
}

fn check(fail: bool) {
    if fail {
        panic!("asked to");
    }
}

/// Every arm moves out all that its variant owns.
#[inline(never)]
fn all_arms(it: Either, fail: bool) -> u32 {
    let n = match it {
        Either::Left(a) => {
            log(format!("left {}", a.0));
            1
        }
        Either::Right(a, b) => {
            log(format!("right {} {}", a.0, b.0));
            drop(b);
            2
        }
        Either::Plain(n) => n,
    };
    check(fail);
    log("end".to_owned());
    n
}

/// One arm leaves a field behind, which the drop where the arms meet has to find.
#[inline(never)]
fn one_left_behind(it: Either, fail: bool) -> u32 {
    let n = match it {
        Either::Left(a) => {
            log(format!("left {}", a.0));
            1
        }
        Either::Right(a, _) => {
            log(format!("right {}", a.0));
            2
        }
        Either::Plain(n) => n,
    };
    check(fail);
    log("end".to_owned());
    n
}

/// One arm moves nothing out.
#[inline(never)]
fn one_untouched(it: Either, fail: bool) -> u32 {
    let n = match it {
        Either::Left(a) => {
            log(format!("left {}", a.0));
            1
        }
        Either::Right(..) => 2,
        Either::Plain(n) => n,
    };
    check(fail);
    log("end".to_owned());
    n
}

/// Moved out on some ways only, and by a guard that may not hold.
#[inline(never)]
fn some_ways(it: Result<Noisy, Noisy>, take: bool, fail: bool) -> u32 {
    let mut n = 0;
    if take {
        n = match it {
            Ok(a) if a.0.len() > 2 => {
                log(format!("long {}", a.0));
                1
            }
            Ok(a) => {
                log(format!("ok {}", a.0));
                2
            }
            Err(e) => {
                log(format!("err {}", e.0));
                3
            }
        };
    }
    check(fail);
    log("end".to_owned());
    n
}

/// The value is made again after it is spent, in a loop.
#[inline(never)]
fn again(names: &[&'static str], fail_at: usize) -> u32 {
    let mut it: Option<Noisy> = None;
    let mut n = 0;
    for (index, name) in names.iter().enumerate() {
        match it {
            Some(a) => {
                log(format!("some {}", a.0));
                n += 1;
            }
            None => n += 10,
        }
        it = Some(Noisy(name));
        check(index == fail_at);
    }
    log("end".to_owned());
    n
}

/// The arms meet inside a closure, as they do in `Result::map_or_else`.
#[inline(never)]
fn through_closures(it: Result<Noisy, Noisy>, fail: bool) -> usize {
    it.map_or_else(
        |e| {
            check(fail);
            e.0.len()
        },
        |a| {
            check(fail);
            a.0.len() + 100
        },
    )
}

fn caught(run: impl FnOnce() -> u32) -> String {
    match catch_unwind(AssertUnwindSafe(run)) {
        Ok(n) => format!("returned {n}"),
        Err(_) => "panicked".to_owned(),
    }
}

pub fn run() -> Vec<String> {
    for fail in [false, true] {
        log(caught(|| all_arms(Either::Left(Noisy("l")), fail)));
        log(caught(|| all_arms(Either::Right(Noisy("r1"), Noisy("r2")), fail)));
        log(caught(|| all_arms(Either::Plain(7), fail)));
        log(caught(|| one_left_behind(Either::Left(Noisy("l")), fail)));
        log(caught(|| one_left_behind(Either::Right(Noisy("r1"), Noisy("r2")), fail)));
        log(caught(|| one_left_behind(Either::Plain(8), fail)));
        log(caught(|| one_untouched(Either::Left(Noisy("l")), fail)));
        log(caught(|| one_untouched(Either::Right(Noisy("r1"), Noisy("r2")), fail)));
        log(caught(|| one_untouched(Either::Plain(9), fail)));
        for take in [false, true] {
            log(caught(|| some_ways(Ok(Noisy("long")), take, fail)));
            log(caught(|| some_ways(Ok(Noisy("ok")), take, fail)));
            log(caught(|| some_ways(Err(Noisy("e")), take, fail)));
        }
        log(caught(|| through_closures(Ok(Noisy("ok")), fail) as u32));
        log(caught(|| through_closures(Err(Noisy("e")), fail) as u32));
    }
    for fail_at in [9, 0, 1, 2] {
        log(caught(|| again(&["a", "b", "c"], fail_at)));
    }
    LOG.with(|log| log.take())
}
