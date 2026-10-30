//! tests/ui/drop/issue-90752-raw-ptr-shenanigans.rs: a local known to be `None` is written
//! through a raw pointer and then partly moved out of in a `match`. rustc's drop elaboration
//! does not see the write, so the part that is not moved is leaked; ra-jit dropped it.

use std::cell::RefCell;

struct Noisy<'a>(u8, &'a RefCell<Vec<u8>>);

impl Drop for Noisy<'_> {
    fn drop(&mut self) {
        self.1.borrow_mut().push(self.0);
    }
}

#[inline(never)]
fn write_behind(log: &RefCell<Vec<u8>>) {
    let mut slot = None;
    let behind: *mut _ = &mut slot;
    match slot {
        None => (),
        _ => return,
    }
    unsafe { *behind = Some((Noisy(0, log), Noisy(1, log))) };
    match slot {
        Some((_first, _)) => {}
        _ => {}
    }
}

pub fn run() -> Vec<u8> {
    let log = RefCell::new(Vec::new());
    write_behind(&log);
    log.into_inner()
}
