//! bun's bundler names its worker `ThreadPool::Worker`, a type alias in an inherent impl of
//! `ThreadPool`, and `run_nested_rename_task` calls `ThreadPool::Worker::get(..)`.
#![feature(inherent_associated_types)]
#![allow(incomplete_features)]

pub struct Worker {
    id: u32,
}

impl Worker {
    pub fn get(pool: &ThreadPool) -> Worker {
        Worker { id: pool.next * 10 }
    }
}

pub struct ThreadPool {
    next: u32,
}

impl ThreadPool {
    pub type Worker = Worker;

    fn spare(&self) -> Self::Worker {
        Self::Worker::get(self)
    }
}

pub struct Slots<T>(pub T);

impl<T: Copy> Slots<T> {
    pub type Slot = T;

    fn first(&self) -> Self::Slot {
        self.0
    }
}

pub fn rename(next: u32) -> u32 {
    let pool = ThreadPool { next };
    let worker = ThreadPool::Worker::get(&pool);
    let slot: Slots<u32>::Slot = Slots(worker.id).first();
    slot + pool.spare().id
}
