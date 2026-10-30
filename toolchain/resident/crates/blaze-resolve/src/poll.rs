//! Running the async [`Registry`](crate::Registry) from the resolver's loop:
//! `src/util/local_poll_adapter.rs` of cargo 1.99.0 (5f94df4789), with an executor of its
//! own in place of the `futures` crate's.

use std::future::Future;
use std::hash::Hash;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::thread::Thread;

use rustc_hash::FxHashMap;

type LocalBoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Caches and runs an async method from code that is not async, on one thread.
///
/// The `self_parameter`, the key and successful results must be cheap to clone. At most one
/// computation per key is in flight; a success is kept, an error is handed out once.
pub(crate) struct LocalPollAdapter<'a, S, K, R> {
    pool: Vec<LocalBoxFuture<'a, (K, R)>>,
    cache: FxHashMap<K, Poll<R>>,
    self_parameter: S,
}

impl<'a, S, K, V, E> LocalPollAdapter<'a, S, K, Result<V, E>>
where
    S: Clone + Deref + 'a,
    K: Clone + Hash + Eq + 'a,
    V: Clone,
{
    pub(crate) fn new(self_parameter: S) -> Self {
        Self { pool: Vec::new(), cache: FxHashMap::default(), self_parameter }
    }

    /// The result for `key`, starting the work if needed. On [`Poll::Pending`], call
    /// [`LocalPollAdapter::wait`] and ask again. Work that finishes without waiting is not
    /// queued.
    pub(crate) fn poll<F>(&mut self, f: F, key: K) -> Poll<Result<V, E>>
    where
        F: AsyncFn(&S::Target, &K) -> Result<V, E> + 'a,
    {
        match self.cache.get(&key) {
            Some(Poll::Ready(Ok(v))) => return Poll::Ready(Ok(v.clone())),
            Some(Poll::Ready(Err(_))) => return self.cache.remove(&key).unwrap(),
            Some(Poll::Pending) => return Poll::Pending,
            None => {}
        }

        let mut future: LocalBoxFuture<'a, (K, Result<V, E>)> = {
            let key = key.clone();
            let self_parameter = self.self_parameter.clone();
            Box::pin(async move {
                let v = f(self_parameter.deref(), &key).await;
                (key, v)
            })
        };

        if let Poll::Ready((k, v)) = future.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            if let Ok(success) = &v {
                self.cache.insert(k, Poll::Ready(Ok(success.clone())));
            }
            return Poll::Ready(v);
        }

        self.cache.insert(key, Poll::Pending);
        self.pool.push(future);
        Poll::Pending
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pool.len()
    }

    /// Runs the pending work to its end. True if there was none.
    pub(crate) fn wait(&mut self) -> bool {
        let is_empty = self.pool.is_empty();
        let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
        let mut cx = Context::from_waker(&waker);
        while !self.pool.is_empty() {
            let mut progressed = false;
            let mut i = 0;
            while i < self.pool.len() {
                if let Poll::Ready((k, v)) = self.pool[i].as_mut().poll(&mut cx) {
                    drop(self.pool.swap_remove(i));
                    *self.cache.get_mut(&k).expect("all pending work is in the cache") =
                        Poll::Ready(v);
                    progressed = true;
                } else {
                    i += 1;
                }
            }
            if !progressed {
                std::thread::park();
            }
        }
        is_empty
    }
}

/// `expect` for a [`Poll`] that must be ready.
pub(crate) trait PollExt<T> {
    fn expect(self, msg: &str) -> T;
}

impl<T> PollExt<T> for Poll<T> {
    #[track_caller]
    fn expect(self, msg: &str) -> T {
        match self {
            Poll::Ready(val) => val,
            Poll::Pending => panic!("{msg}"),
        }
    }
}

/// Runs `future` to its end on this thread.
pub(crate) fn block_on<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    loop {
        if let Poll::Ready(v) = future.as_mut().poll(&mut cx) {
            return v;
        }
        std::thread::park();
    }
}

/// Wakes the thread that waits; a wake before it parks makes the park return at once.
struct Unpark(Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }

    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::rc::Rc;

    use super::*;

    /// A future that is pending once, as a query waiting on the network is.
    pub(crate) fn yield_once() -> impl Future<Output = ()> {
        let mut yielded = false;
        std::future::poll_fn(move |cx| {
            if yielded {
                Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
    }

    /// A future woken from another thread, as one waiting on a download is. Each poll leaves
    /// its waker for the thread, as a future must.
    fn woken_from_elsewhere() -> impl Future<Output = ()> {
        let state = Arc::new(std::sync::Mutex::new((false, None::<Waker>)));
        let mut started = false;
        std::future::poll_fn(move |cx| {
            let mut guard = state.lock().unwrap();
            if guard.0 {
                return Poll::Ready(());
            }
            guard.1 = Some(cx.waker().clone());
            if !started {
                started = true;
                let state = state.clone();
                std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(20));
                    let mut guard = state.lock().unwrap();
                    guard.0 = true;
                    guard.1.take().unwrap().wake();
                });
            }
            Poll::Pending
        })
    }

    struct Thing {}

    impl Thing {
        async fn widen(&self, i: &i32) -> Result<i64, ()> {
            if *i > 1000 {
                woken_from_elsewhere().await;
            } else if *i > 10 {
                yield_once().await;
            }
            if *i % 2 != 0 {
                return Err(());
            }
            Ok(*i as i64)
        }
    }

    struct PolledThing<'a> {
        poller: LocalPollAdapter<'a, Rc<Thing>, i32, Result<i64, ()>>,
    }

    impl PolledThing<'_> {
        fn new() -> Self {
            Self { poller: LocalPollAdapter::new(Rc::new(Thing {})) }
        }

        fn widen(&mut self, i: &i32) -> Poll<Result<i64, ()>> {
            self.poller.poll(Thing::widen, *i)
        }

        fn wait(&mut self) -> bool {
            self.poller.wait()
        }
    }

    #[test]
    fn immediate_success() {
        let mut p = PolledThing::new();
        assert_eq!(p.widen(&2), Poll::Ready(Ok(2)));
        assert!(p.wait());
    }

    #[test]
    fn immediate_error() {
        let mut p = PolledThing::new();
        assert_eq!(p.widen(&1), Poll::Ready(Err(())));
        assert!(p.wait());
    }

    #[test]
    fn deferred_error() {
        let mut p = PolledThing::new();
        assert_eq!(p.widen(&101), Poll::Pending);
        assert!(!p.wait());
        assert_eq!(p.widen(&101), Poll::Ready(Err(())));
        assert!(p.wait());
        // Errors are not kept.
        assert_eq!(p.widen(&101), Poll::Pending);
        assert!(!p.wait());
        assert_eq!(p.widen(&101), Poll::Ready(Err(())));
        assert!(p.wait());
    }

    #[test]
    fn deferred_success() {
        let mut p = PolledThing::new();
        assert_eq!(p.widen(&50), Poll::Pending);
        assert!(!p.wait());
        assert_eq!(p.widen(&50), Poll::Ready(Ok(50)));
        assert!(p.wait());
        // Success is kept.
        assert_eq!(p.widen(&50), Poll::Ready(Ok(50)));
        assert!(p.wait());
    }

    #[test]
    fn waits_for_another_thread() {
        let mut p = PolledThing::new();
        assert_eq!(p.widen(&2000), Poll::Pending);
        assert_eq!(p.widen(&50), Poll::Pending);
        assert_eq!(p.poller.pending_count(), 2);
        assert!(!p.wait());
        assert_eq!(p.widen(&2000), Poll::Ready(Ok(2000)));
        assert_eq!(p.widen(&50), Poll::Ready(Ok(50)));
    }
}
