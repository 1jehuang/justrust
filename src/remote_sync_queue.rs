//! Foreground admission for the source-sync mutex. Correctness still requires
//! one owner of a mirror at a time, but optional watcher batches must yield
//! between roots to builds already waiting for that owner.
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, LockResult, Mutex, MutexGuard, TryLockResult};
use std::time::{Duration, Instant};

/// A failed connect this long is the answer for the builds that waited for
/// it too, instead of being repeated by each of them. A quick failure is
/// cheap to retry and may have been a blip.
const SHARE_FAILURE_AFTER: Duration = Duration::from_secs(10);

/// One connect to the machine, made without the mirror lock. Whoever finds
/// no link while one is being made waits for this instead of the lock.
pub(super) struct Attempt {
    began: Instant,
    /// Whether this attempt may start a stopped machine.
    start: bool,
    /// `Ok` once the link is published, with how long the attempt took.
    done: Mutex<Option<(Duration, Result<(), String>)>>,
    ended: Condvar,
}

/// How a connect someone else made ended for a build that waited for it.
#[derive(Debug, PartialEq)]
pub(super) enum Followed {
    /// A link was published: look for it under the lock.
    Connected,
    /// It failed after long enough that this build should not repeat it.
    Failed(String),
    /// It failed quickly, or could not do what this build may (start the
    /// machine): make another attempt.
    Retry,
    /// Still not finished at the bound, after this many seconds.
    Overdue(u64),
}

impl Attempt {
    /// Wait for the attempt to end, at most until it is `bound` old.
    pub(super) fn follow(&self, start: bool, bound: Option<Duration>) -> Followed {
        self.follow_sharing(start, bound, SHARE_FAILURE_AFTER)
    }

    fn follow_sharing(&self, start: bool, bound: Option<Duration>, slow: Duration) -> Followed {
        let Ok(mut done) = self.done.lock() else {
            return Followed::Retry;
        };
        loop {
            if let Some((took, outcome)) = done.as_ref() {
                return match outcome {
                    Ok(()) => Followed::Connected,
                    Err(why) if *took >= slow && (self.start || !start) => {
                        Followed::Failed(why.clone())
                    }
                    Err(_) => Followed::Retry,
                };
            }
            let age = self.began.elapsed();
            let left = match bound {
                Some(bound) if age >= bound => return Followed::Overdue(age.as_secs()),
                Some(bound) => bound - age,
                None => Duration::from_secs(3600),
            };
            done = match self.ended.wait_timeout(done, left) {
                Ok((done, _)) => done,
                Err(_) => return Followed::Retry,
            };
        }
    }
}

/// The one connect in flight, held by whoever makes it. Dropped without an
/// outcome (a panic in the connect) it ends as a failure: nobody waits for
/// an attempt that no longer exists.
pub(super) struct Lead<'a> {
    slot: &'a Mutex<Option<Arc<Attempt>>>,
    attempt: Arc<Attempt>,
    finished: bool,
}

impl Lead<'_> {
    pub(super) fn finish(mut self, outcome: Result<(), String>) {
        self.end(outcome);
    }

    fn end(&mut self, outcome: Result<(), String>) {
        if std::mem::replace(&mut self.finished, true) {
            return;
        }
        // Out of the slot first: whoever wakes up must be able to lead.
        if let Ok(mut slot) = self.slot.lock()
            && slot.as_ref().is_some_and(|a| Arc::ptr_eq(a, &self.attempt))
        {
            *slot = None;
        }
        if let Ok(mut done) = self.attempt.done.lock() {
            *done = Some((self.attempt.began.elapsed(), outcome));
        }
        self.attempt.ended.notify_all();
    }
}

impl Drop for Lead<'_> {
    fn drop(&mut self) {
        self.end(Err("the connect was abandoned".into()));
    }
}

pub(super) enum Turn<'a> {
    /// Nobody is connecting: this caller does, and must `finish`.
    Lead(Lead<'a>),
    /// Someone is: wait for theirs.
    Follow(Arc<Attempt>),
}

pub(super) struct SyncQueue<T> {
    inner: Mutex<T>,
    foreground_epoch: AtomicUsize,
    foreground_waiters: AtomicUsize,
    connect: Mutex<Option<Arc<Attempt>>>,
}

impl<T> SyncQueue<T> {
    pub(super) fn new(value: T) -> Self {
        Self {
            inner: Mutex::new(value),
            foreground_epoch: AtomicUsize::new(0),
            foreground_waiters: AtomicUsize::new(0),
            connect: Mutex::default(),
        }
    }

    /// Whose turn it is to connect. Call it holding the mirror lock, having
    /// found no live link, and release that lock before connecting or
    /// following: a link is only ever published under the lock, so no
    /// attempt can end unseen between the look and the turn.
    pub(super) fn connect_turn(&self, start: bool) -> Turn<'_> {
        let mut slot = self.connect.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(attempt) = slot.as_ref() {
            return Turn::Follow(attempt.clone());
        }
        let attempt = Arc::new(Attempt {
            began: Instant::now(),
            start,
            done: Mutex::new(None),
            ended: Condvar::new(),
        });
        *slot = Some(attempt.clone());
        Turn::Lead(Lead {
            slot: &self.connect,
            attempt,
            finished: false,
        })
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

    pub(super) fn foreground_epoch(&self) -> usize {
        self.foreground_epoch.load(Ordering::SeqCst)
    }

    /// A build is waiting for the lock: background work should yield.
    pub(super) fn foreground_waiting(&self) -> bool {
        self.foreground_waiters.load(Ordering::SeqCst) != 0
    }

    pub(super) fn foreground_lock(&self) -> LockResult<MutexGuard<'_, T>> {
        self.foreground_waiters.fetch_add(1, Ordering::SeqCst);
        let guard = self.inner.lock();
        self.foreground_epoch.fetch_add(1, Ordering::SeqCst);
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

    /// 2026-11-27: one connect held the mirror lock for 427 s and eleven
    /// builds queued behind it for up to 482 s, each then free to try again.
    /// The connect is now made without that lock, by one caller at a time.
    #[test]
    fn one_connect_is_made_without_the_lock_and_everyone_waiting_gets_its_end() {
        let queue = Arc::new(SyncQueue::new(0u32));
        let slow = Duration::from_millis(50);
        // The first build finds no link, takes the turn and lets the lock go.
        let guard = queue.foreground_lock().unwrap();
        let Turn::Lead(lead) = queue.connect_turn(false) else {
            panic!("nobody else is connecting");
        };
        drop(guard);
        // Eleven builds arrive while it is stuck. None of them waits for the
        // lock, and none starts a second connect.
        let waiters: Vec<_> = (0..11)
            .map(|i| {
                let queue = queue.clone();
                std::thread::spawn(move || {
                    let asked = Instant::now();
                    let guard = queue.foreground_lock().unwrap();
                    let locked = asked.elapsed();
                    let Turn::Follow(attempt) = queue.connect_turn(false) else {
                        panic!("a second connect was started");
                    };
                    drop(guard);
                    // Five have a bound the stuck connect outlives.
                    let bound = (i < 5).then(|| Duration::from_millis(150));
                    (attempt.follow_sharing(false, bound, slow), locked)
                })
            })
            .collect();
        std::thread::sleep(Duration::from_millis(400));
        // Work that needs only the lock is not held up by the connect.
        *queue.foreground_lock().unwrap() += 1;
        lead.finish(Err("handshake timed out".into()));
        let ends: Vec<_> = waiters.into_iter().map(|w| w.join().unwrap()).collect();
        let count = |f: fn(&Followed) -> bool| ends.iter().filter(|(e, _)| f(e)).count();
        assert_eq!(
            (
                count(|e| matches!(e, Followed::Overdue(_))),
                count(|e| *e == Followed::Failed("handshake timed out".into())),
            ),
            (5, 6),
            "{ends:?}"
        );
        assert!(
            ends.iter()
                .all(|(_, locked)| *locked < Duration::from_millis(300)),
            "the lock was free during the connect: {ends:?}"
        );
        assert_eq!(*queue.lock().unwrap(), 1);

        // The turn is free again. A quick failure is retried by whoever
        // followed it, and so is one that could not start the machine when
        // the follower may.
        let Turn::Lead(lead) = queue.connect_turn(false) else {
            panic!("the failed attempt left the turn taken");
        };
        let Turn::Follow(attempt) = queue.connect_turn(true) else {
            panic!("a second connect was started");
        };
        lead.finish(Err("remote machine is not running".into()));
        assert_eq!(attempt.follow_sharing(false, None, slow), Followed::Retry);
        assert_eq!(
            attempt.follow_sharing(false, None, Duration::ZERO),
            Followed::Failed("remote machine is not running".into())
        );
        assert_eq!(
            attempt.follow_sharing(true, None, Duration::ZERO),
            Followed::Retry
        );
        // A published link is everyone's.
        let Turn::Lead(lead) = queue.connect_turn(true) else {
            panic!("the turn is free");
        };
        let Turn::Follow(attempt) = queue.connect_turn(true) else {
            panic!("a second connect was started");
        };
        lead.finish(Ok(()));
        assert_eq!(attempt.follow(true, None), Followed::Connected);
        // A connect that dies without an outcome frees the turn and wakes
        // its followers.
        let Turn::Lead(lead) = queue.connect_turn(true) else {
            panic!("the turn is free");
        };
        let Turn::Follow(attempt) = queue.connect_turn(true) else {
            panic!("a second connect was started");
        };
        drop(lead);
        assert_eq!(
            attempt.follow_sharing(true, None, Duration::ZERO),
            Followed::Failed("the connect was abandoned".into())
        );
        assert!(matches!(queue.connect_turn(true), Turn::Lead(_)));
    }

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
    #[ignore = "diagnostic: scans the current workspace twice"]
    fn watcher_scan_contention() {
        for speculative in [false, true] {
            let queue = Arc::new(SyncQueue::new(()));
            let guard = queue.background_lock().unwrap();
            let epoch = queue.foreground_epoch();
            let other = queue.clone();
            let foreground = std::thread::spawn(move || {
                let start = std::time::Instant::now();
                let _guard = other.foreground_lock().unwrap();
                start.elapsed()
            });
            while queue.foreground_waiters.load(Ordering::SeqCst) == 0 {
                std::thread::yield_now();
            }
            let guard = if speculative {
                drop(guard);
                None
            } else {
                Some(guard)
            };
            let scan = crate::remote_sync::scan(&std::env::current_dir().unwrap()).unwrap();
            if !speculative {
                assert_eq!(epoch, queue.foreground_epoch());
            }
            drop(guard);
            let wait = foreground.join().unwrap();
            let _commit = queue.background_lock().unwrap();
            assert_ne!(
                epoch,
                queue.foreground_epoch(),
                "stale scan must be rejected"
            );
            eprintln!(
                "watcher scan contention speculative={speculative} paths={} foreground_wait_ms={:.3}",
                scan.files.len(),
                wait.as_secs_f64() * 1000.0
            );
        }
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
