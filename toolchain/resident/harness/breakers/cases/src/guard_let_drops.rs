//! tests/ui/drop/if-let-guards.rs: what an `if let` guard binds and its temporaries are
//! dropped before the bindings of the arm's pattern, and at once when the guard fails.

use std::cell::RefCell;

struct Noisy<'a>(u8, &'a RefCell<Vec<u8>>);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

#[inline(never)]
fn guard_succeeds(log: &RefCell<Vec<u8>>) {
    match [Noisy(6, log), Noisy(5, log)] {
        [_x, _y] if let [_, _z, _] = [Noisy(4, log), Noisy(2, log), Noisy(3, log)] && true => {
            let _a = Noisy(1, log);
        }
        _ => unreachable!(),
    }
}

#[inline(never)]
fn guard_fails(log: &RefCell<Vec<u8>>) {
    match [Noisy(13, log), Noisy(14, log)] {
        [_x, _y] if let _z = Noisy(11, log) && false => unreachable!(),
        _ => {
            let _a = Noisy(12, log);
        }
    }
}

#[inline(never)]
fn guards_in_turn(log: &RefCell<Vec<u8>>) {
    let mut count = 21;
    match Noisy(28, log) {
        _ | _ | _ if let _ = Noisy(count, log) && { count += 1; false } => unreachable!(),
        _ if let _ = Noisy(25, log) && { log.borrow_mut().push(24); false } => unreachable!(),
        _ if let _ = Noisy(27, log) && true => {
            Noisy(26, log);
        }
        _ => unreachable!(),
    }
}

#[inline(never)]
fn refuses(log: &RefCell<Vec<u8>>) -> bool {
    log.borrow_mut().push(31);
    false
}

/// The arm that follows a failed guard finds the array whole, and so it is dropped from the front.
#[inline(never)]
fn array_whole_again(log: &RefCell<Vec<u8>>) {
    match [Noisy(33, log), Noisy(34, log), Noisy(35, log)] {
        [_x, _, _y] if refuses(log) => unreachable!(),
        _ => {
            let _a = Noisy(32, log);
        }
    }
}

pub fn run() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    guard_succeeds(&log);
    guard_fails(&log);
    guards_in_turn(&log);
    array_whole_again(&log);
    log.into_inner()
}
