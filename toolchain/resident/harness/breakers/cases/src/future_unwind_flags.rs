//! tests/ui/async-await/async-fn-size-moved-locals.rs and async-fn-size-uninit-locals.rs: the
//! size of a future whose local holds a value at one call that may unwind and none at another.
//! rustc's cleanup for the two calls shares the drop of the local, so it keeps a flag saying
//! whether there is a value, and the flag is a byte of the future when a call after the await
//! can unwind into that cleanup. ra-jit drops by where the call is and keeps no such byte.

use std::future::Future;
use std::mem::size_of_val;
use std::pin::Pin;
use std::task::{Context, Poll};

struct Held([u8; 16]);

impl Drop for Held {
    fn drop(&mut self) {}
}

impl Future for Held {
    type Output = ();
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<()> {
        Poll::Ready(())
    }
}

#[inline(never)]
fn call() {}

async fn nothing() {}

async fn moved() {
    let held = Held([0; 16]);
    held.await;
}

async fn moved_after_call() {
    let held = Held([0; 16]);
    call();
    held.await;
}

async fn given_late() {
    let held;
    nothing().await;
    held = Held([0; 16]);
}

async fn given_between_calls() {
    let held;
    nothing().await;
    call();
    held = Held([0; 16]);
    call();
}

pub fn run() -> Vec<usize> {
    vec![
        size_of_val(&moved()),
        size_of_val(&moved_after_call()),
        size_of_val(&given_late()),
        size_of_val(&given_between_calls()),
    ]
}
