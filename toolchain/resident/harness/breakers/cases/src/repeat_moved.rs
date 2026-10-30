//! tests/ui/drop/repeat-drop.rs: `[value; 0]` drops the value there and `[value; 1]` moves it
//! into the array; the value was dropped a second time at the end of the scope. A constant is
//! made once per element, so `[CONSTANT; 0]` makes and drops none. When the drop of `[value; 0]`
//! panics, what was built before it is dropped first. library/coretests slice.rs: an array of
//! `usize::MAX` elements that take no room is made at once; a write was looped over each.

use std::cell::RefCell;

struct Noisy<'a>(u8, &'a RefCell<Vec<u8>>);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

#[inline(never)]
fn none(log: &RefCell<Vec<u8>>) {
    let value = Noisy(1, log);
    let array: [Noisy; 0] = [value; 0];
    log.borrow_mut().push(10);
    drop(array);
    log.borrow_mut().push(11);
}

#[inline(never)]
fn one(log: &RefCell<Vec<u8>>) {
    let value = Noisy(2, log);
    let array: [Noisy; 1] = [value; 1];
    log.borrow_mut().push(20);
    drop(array);
    log.borrow_mut().push(21);
}

struct Loud;

impl Drop for Loud {
    fn drop(&mut self) {
        panic!("a constant of an empty array was dropped");
    }
}

const LOUD: Loud = Loud;

#[inline(never)]
fn constant<const N: usize>() -> usize {
    let listed: [Loud; 0] = [LOUD; 0];
    let counted: [Loud; N] = [LOUD; N];
    let block: [Loud; 0] = [const { Loud }; 0];
    listed.len() + counted.len() + block.len()
}

struct Bursting;

impl Drop for Bursting {
    fn drop(&mut self) {
        std::panic::resume_unwind(Box::new(()));
    }
}

#[inline(never)]
fn bursting(log: &RefCell<Vec<u8>>) {
    let value = Bursting;
    let _local = Noisy(31, log);
    let _pair = (Noisy(30, log), [value; 0]);
    std::process::abort();
}

#[inline(never)]
fn roomless(unit: ()) -> usize {
    let units: &[()] = &[unit; usize::MAX];
    let constant: &[()] = &[(); usize::MAX];
    (units.len() == constant.len()) as usize + units.array_windows::<1>().count() % 7
}

pub fn run() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    none(&log);
    one(&log);
    log.borrow_mut().push(constant::<0>() as u8);
    log.borrow_mut().push(roomless(()) as u8);
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| bursting(&log)));
    log.borrow_mut().push(caught.is_err() as u8);
    log.into_inner()
}
