//! spin's tests of a lock taken while unwinding bind a guard and panic: the guard is built in
//! a temporary and moved to a binding that nothing reads afterwards.

use std::sync::{Arc, Mutex};

struct Unwinder {
    count: Arc<Mutex<isize>>,
}

impl Drop for Unwinder {
    fn drop(&mut self) {
        *self.count.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) += 1;
    }
}

pub fn run() -> (bool, isize) {
    let count = Arc::new(Mutex::new(1));
    let shared = count.clone();
    let joined = std::thread::spawn(move || -> () {
        let _unwinder = Unwinder { count: shared };
        panic!();
    })
    .join();
    let count = *count.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    (joined.is_err(), count)
}
