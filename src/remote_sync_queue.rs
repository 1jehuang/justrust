//! Foreground admission for the source-sync mutex. Correctness still requires
//! one owner of a mirror at a time, but optional watcher batches must yield
//! between roots to builds already waiting for that owner.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{LockResult, Mutex, MutexGuard, TryLockResult};

pub(super) struct SyncQueue<T> {
    inner: Mutex<T>,
    foreground_waiters: AtomicUsize,
}

impl<T> SyncQueue<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(value),
            foreground_waiters: AtomicUsize::new(0),
        }
    }

    #[cfg(test)]
    pub(super) fn lock(&self) -> LockResult<MutexGuard<'_, T>> {
        self.inner.lock()
    }

    pub(super) fn try_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        self.inner.try_lock()
    }

    /// Optional connection warmup must never join the mirror queue or overtake
    /// a build already waiting. Check again after acquiring to cover a build
    /// registering between the initial check and the nonblocking lock attempt.
    pub(super) fn try_background_lock(&self) -> TryLockResult<MutexGuard<'_, T>> {
        use std::sync::TryLockError;
        if self.foreground_waiters.load(Ordering::SeqCst) != 0 {
            return Err(TryLockError::WouldBlock);
        }
        let guard = self.inner.try_lock()?;
        if self.foreground_waiters.load(Ordering::SeqCst) != 0 {
            return Err(TryLockError::WouldBlock);
        }
        Ok(guard)
    }

    pub(super) fn foreground_lock(&self) -> LockResult<MutexGuard<'_, T>> {
        self.foreground_waiters.fetch_add(1, Ordering::SeqCst);
        let guard = self.inner.lock();
        self.foreground_waiters.fetch_sub(1, Ordering::SeqCst);
        guard
    }

    pub(super) fn background_lock(&self) -> LockResult<MutexGuard<'_, T>> {
        loop {
            let guard = self.inner.lock()?;
            if self.foreground_waiters.load(Ordering::SeqCst) == 0 {
                return Ok(guard);
            }
            drop(guard);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;

    #[test]
    fn queued_foreground_precedes_remaining_background_roots() {
        let queue = Arc::new(SyncQueue::new(Vec::new()));
        let first_root = queue.background_lock().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let foreground_queue = queue.clone();
        let foreground = std::thread::spawn(move || {
            foreground_queue
                .foreground_lock()
                .unwrap()
                .push("foreground");
            done_tx.send(()).unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while queue.foreground_waiters.load(Ordering::SeqCst) == 0 {
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        }
        let background_queue = queue.clone();
        let background = std::thread::spawn(move || {
            for _ in 0..4 {
                background_queue
                    .background_lock()
                    .unwrap()
                    .push("background");
            }
        });
        drop(first_root);
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        foreground.join().unwrap();
        background.join().unwrap();
        assert_eq!(
            &*queue.lock().unwrap(),
            &[
                "foreground",
                "background",
                "background",
                "background",
                "background"
            ]
        );
        assert_eq!(queue.foreground_waiters.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn foreground_batch_contention_acceptance() {
        fn measure(yield_between_roots: bool) -> (Duration, usize) {
            let queue = Arc::new(SyncQueue::new(Vec::<usize>::new()));
            let (ready_tx, ready_rx) = mpsc::channel();
            let (resume_tx, resume_rx) = mpsc::channel();
            let background_queue = queue.clone();
            let background = std::thread::spawn(move || {
                let mut initial = background_queue.background_lock().unwrap();
                ready_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                if yield_between_roots {
                    initial.push(0);
                    std::thread::sleep(Duration::from_millis(40));
                    drop(initial);
                    for root in 1..4 {
                        let mut guard = background_queue.background_lock().unwrap();
                        guard.push(root);
                        std::thread::sleep(Duration::from_millis(40));
                    }
                } else {
                    for root in 0..4 {
                        initial.push(root);
                        std::thread::sleep(Duration::from_millis(40));
                    }
                }
            });
            ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            let foreground_queue = queue.clone();
            let foreground = std::thread::spawn(move || {
                let start = std::time::Instant::now();
                let guard = foreground_queue.foreground_lock().unwrap();
                (start.elapsed(), guard.len())
            });
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while queue.foreground_waiters.load(Ordering::SeqCst) == 0 {
                assert!(std::time::Instant::now() < deadline);
                std::thread::yield_now();
            }
            resume_tx.send(()).unwrap();
            let observation = foreground.join().unwrap();
            background.join().unwrap();
            // All pending roots eventually execute exactly once, in order.
            assert_eq!(&*queue.lock().unwrap(), &[0, 1, 2, 3]);
            observation
        }
        let (control_wait, control_completed) = measure(false);
        let (priority_wait, priority_completed) = measure(true);
        assert_eq!(control_completed, 4);
        assert_eq!(priority_completed, 1);
        eprintln!(
            "sync_queue diagnostic: batched_control_ms={:.3} foreground_priority_ms={:.3}; paced mutex control, not an end-to-end build speedup",
            control_wait.as_secs_f64() * 1000.0,
            priority_wait.as_secs_f64() * 1000.0,
        );
    }

    #[test]
    fn optional_warmup_never_queues_behind_a_busy_mirror() {
        let queue = Arc::new(SyncQueue::new(()));
        let held = queue.lock().unwrap();
        let (tx, rx) = mpsc::channel();
        let other = queue.clone();
        let worker = std::thread::spawn(move || {
            tx.send(matches!(
                other.try_background_lock(),
                Err(std::sync::TryLockError::WouldBlock)
            ))
            .unwrap();
        });
        // The owner remains locked until after the optional call has returned.
        let result = rx.recv_timeout(Duration::from_secs(5));
        drop(held);
        worker.join().unwrap();
        assert!(result.unwrap());
        assert!(queue.try_background_lock().is_ok());
    }

    #[test]
    fn optional_warmup_yields_even_when_mirror_is_free_but_build_is_queued() {
        let queue = SyncQueue::new(());
        queue.foreground_waiters.store(1, Ordering::SeqCst);
        assert!(matches!(
            queue.try_background_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));
        // A declined warmup must not retain the mirror lock.
        assert!(queue.try_lock().is_ok());
        queue.foreground_waiters.store(0, Ordering::SeqCst);
        assert!(queue.try_background_lock().is_ok());
    }

    #[test]
    fn poisoned_foreground_acquisition_does_not_leave_waiter_registered() {
        let queue = Arc::new(SyncQueue::new(()));
        let other = queue.clone();
        assert!(
            std::thread::spawn(move || {
                let _guard = other.lock().unwrap();
                panic!("poison fixture");
            })
            .join()
            .is_err()
        );
        assert!(queue.foreground_lock().is_err());
        assert_eq!(queue.foreground_waiters.load(Ordering::SeqCst), 0);
        assert!(queue.background_lock().is_err());
        assert!(matches!(
            queue.try_background_lock(),
            Err(std::sync::TryLockError::Poisoned(_))
        ));
    }
}
