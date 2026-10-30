//! tests/ui/async-await/drop-order/drop-order-for-async-fn-parameters-by-ref-binding.rs: an
//! `async fn` with a `ref` parameter moves the argument into a local of its block and binds
//! the name to that. The capture analysis took the local for an argument the block captures,
//! so the function making the future looked for a variable it does not have.

use std::cell::RefCell;
use std::future::Future;
use std::pin::pin;
use std::task::{Context, Poll, Waker};

thread_local!(static ORDER: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) });

struct Noted(&'static str);

impl Drop for Noted {
    fn drop(&mut self) {
        ORDER.with(|it| it.borrow_mut().push(self.0));
    }
}

async fn by_ref(ref mut first: Noted, ref second: Noted) -> usize {
    first.0 = "changed";
    first.0.len() + second.0.len()
}

async fn in_tuple(ref whole: Noted, (ref mut left, _): (Noted, Noted), _: Noted) -> usize {
    left.0 = "left";
    whole.0.len()
}

fn ready<T>(future: impl Future<Output = T>) -> T {
    let mut future = pin!(future);
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(it) => it,
        Poll::Pending => unreachable!(),
    }
}

pub fn run() -> (usize, usize, Vec<&'static str>) {
    let sum = ready(by_ref(Noted("first"), Noted("second")));
    let whole = ready(in_tuple(Noted("whole"), (Noted("l"), Noted("right")), Noted("last")));
    (sum, whole, ORDER.with(|it| it.take()))
}
