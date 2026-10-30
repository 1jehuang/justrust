//! async-trait's tests of methods that name the items of their trait through `Self`, one of
//! them with the unit struct `Self` as a pattern.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};

trait Named: Sync {
    const NAME: &'static str;
    type Length: From<u8> + Into<u32> + Send;

    fn length<'a>(&'a self, depth: u8) -> Pin<Box<dyn Future<Output = u32> + Send + 'a>> {
        Box::pin(async move {
            let _: Self::Length;
            Self::NAME;
            let _ = Self::length(self, depth);
            let length: Self::Length = depth.into();
            match depth {
                0 => Self::NAME.len() as u32,
                _ => <Self as Named>::length(self, depth - 1).await + length.into(),
            }
        })
    }
}

struct Unit;

impl Named for Unit {
    const NAME: &'static str = "unit";
    type Length = u16;

    fn length<'a>(&'a self, depth: u8) -> Pin<Box<dyn Future<Output = u32> + Send + 'a>> {
        Box::pin(async move {
            let Self: Self = *self;
            depth as u32
        })
    }
}

struct Pair;

impl Named for Pair {
    const NAME: &'static str = "pair";
    type Length = u16;
}

fn ready(mut future: Pin<Box<dyn Future<Output = u32> + Send + '_>>) -> u32 {
    match future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(length) => length,
        Poll::Pending => 0,
    }
}

pub fn run() -> (u32, u32) {
    (ready(Unit.length(3)), ready(Pair.length(3)))
}
