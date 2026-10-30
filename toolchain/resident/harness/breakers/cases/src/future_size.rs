//! The size of a future is what a program sees of how it is laid out: futures' `join_size`,
//! `select_size`, `try_join_size` and `select_on_non_unpin_size` assert the size of an `async`
//! block. ra-jit laid one out with a word of drop flags for every local held across an await and
//! a word of state; it lays it out as rustc does now, a variant for each await.

use std::future::{ready, Future, Ready};
use std::mem::size_of_val;
use std::pin::Pin;
use std::task::{Context, Poll};

/// Two futures polled in turn, as `join!` holds them.
struct Both<A: Future, B: Future>(Result<A::Output, A>, Result<B::Output, B>);

impl<A: Future + Unpin, B: Future + Unpin> Future for Both<A, B>
where
    A::Output: Unpin + Copy,
    B::Output: Unpin + Copy,
{
    type Output = (A::Output, B::Output);
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        if let Err(future) = &mut this.0 {
            if let Poll::Ready(output) = Pin::new(future).poll(cx) {
                this.0 = Ok(output);
            }
        }
        if let Err(future) = &mut this.1 {
            if let Poll::Ready(output) = Pin::new(future).poll(cx) {
                this.1 = Ok(output);
            }
        }
        match (&this.0, &this.1) {
            (Ok(a), Ok(b)) => Poll::Ready((*a, *b)),
            _ => Poll::Pending,
        }
    }
}

async fn number(n: i32) -> i32 {
    n
}

pub fn run() -> Vec<usize> {
    let one = async {
        let ready: Ready<i32> = ready(0);
        ready.await
    };
    let two = async {
        let (a, b) = (ready(0i32), ready(0i32));
        Both(Err(a), Err(b)).await
    };
    let nested = async {
        let first = number(1).await;
        let second = number(2).await;
        [first, second, number(3).await]
    };
    let held = async {
        let buffer = [1u8; 100];
        number(4).await;
        buffer.len()
    };
    vec![size_of_val(&one), size_of_val(&two), size_of_val(&nested), size_of_val(&held)]
}
