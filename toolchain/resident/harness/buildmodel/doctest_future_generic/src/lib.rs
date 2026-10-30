//! A future a function without type parameters makes, and one with them waits for: the
//! example has rustc's copy of the second, which lays the future out its own way.
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

pub struct Twice(bool);

impl Future for Twice {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            return Poll::Ready(());
        }
        self.0 = true;
        cx.waker().wake_by_ref();
        Poll::Pending
    }
}

pub struct Shared {
    pub queue: Vec<u32>,
    pub spare: Vec<u32>,
}

pub struct Queue<'a> {
    pub shared: &'a mut Shared,
    pub local: Vec<u32>,
    pub ticks: usize,
}

async fn search(mut with: impl FnMut() -> Option<u32>) -> Box<u32> {
    loop {
        match with() {
            Some(found) => return Box::new(found),
            None => Twice(false).await,
        }
    }
}

impl Queue<'_> {
    async fn next(&mut self, skip: &mut u32) -> u32 {
        let found = search(|| {
            if let Some(found) = self.local.pop() {
                return Some(found);
            }
            if let Some(found) = self.shared.queue.pop() {
                self.local.extend(self.shared.spare.drain(..));
                return Some(found);
            }
            *skip += 1;
            (*skip > 2).then_some(0)
        })
        .await;
        self.ticks = self.ticks.wrapping_add(1);
        if self.ticks % 2 == 0 {
            self.local.push(*found);
        }
        *found
    }

    /// ```
    /// use std::future::Future;
    /// use std::task::{Context, Poll, Waker};
    /// let mut shared = app::Shared { queue: vec![1, 2], spare: vec![7] };
    /// let mut queue = app::Queue { shared: &mut shared, local: Vec::new(), ticks: 0 };
    /// let mut run = std::pin::pin!(queue.sum("x"));
    /// let mut cx = Context::from_waker(Waker::noop());
    /// let done = loop {
    ///     if let Poll::Ready(done) = run.as_mut().poll(&mut cx) {
    ///         break done;
    ///     }
    /// };
    /// assert_eq!(done, (vec![2, 7, 7, 1, 1], 0, "x"));
    /// ```
    pub async fn sum<T>(&mut self, with: T) -> (Vec<u32>, u32, T) {
        let mut skip = 0;
        let mut all = Vec::new();
        for _ in 0..5 {
            all.push(self.next(&mut skip).await);
        }
        (all, skip, with)
    }
}
