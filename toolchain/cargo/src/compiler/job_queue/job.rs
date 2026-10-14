//! See [`Job`] and [`Work`].

use std::fmt;
use std::mem;

use super::JobState;
use crate::compiler::fingerprint::DirtyReason;
use crate::compiler::fingerprint::jr_cutoff::CutoffPlan;
use crate::util::CargoResult;

/// Represents a unit of [`Work`] with a [`Freshness`] for caller
/// to determine whether to re-execute or not.
pub struct Job {
    work: Work,
    fresh: Freshness,
    /// justrust: a dirty job that may turn out fresh once its dependencies
    /// finished (cross-crate early cutoff). Resolved by the job queue when
    /// the job is dequeued, see [`Job::resolve_cutoff`].
    cutoff: Option<(CutoffPlan, Box<Job>)>,
}

/// The basic unit of work.
///
/// Each proc should send its description before starting.
/// It should send either once or close immediately.
pub struct Work {
    inner: Box<dyn FnOnce(&JobState<'_, '_>) -> CargoResult<()> + Send>,
}

impl Work {
    /// Creates a unit of work.
    pub fn new<F>(f: F) -> Work
    where
        F: FnOnce(&JobState<'_, '_>) -> CargoResult<()> + Send + 'static,
    {
        Work { inner: Box::new(f) }
    }

    /// Creates a unit of work that does nothing.
    pub fn noop() -> Work {
        Work::new(|_| Ok(()))
    }

    /// Consumes this work by running it.
    pub fn call(self, tx: &JobState<'_, '_>) -> CargoResult<()> {
        (self.inner)(tx)
    }

    /// Creates a new unit of work that chains `next` after ourself.
    pub fn then(self, next: Work) -> Work {
        Work::new(move |state| {
            self.call(state)?;
            next.call(state)
        })
    }
}

impl Job {
    /// Creates a new job that does nothing.
    pub fn new_fresh() -> Job {
        Job {
            work: Work::noop(),
            fresh: Freshness::Fresh,
            cutoff: None,
        }
    }

    /// Creates a new job representing a unit of work.
    pub fn new_dirty(work: Work, dirty_reason: DirtyReason) -> Job {
        Job {
            work,
            fresh: Freshness::Dirty(dirty_reason),
            cutoff: None,
        }
    }

    /// justrust: a fresh job that still runs some bookkeeping `work`.
    pub fn new_fresh_with(work: Work) -> Job {
        Job {
            work,
            fresh: Freshness::Fresh,
            cutoff: None,
        }
    }

    /// justrust: attaches a fresh alternative that replaces this job if the
    /// cutoff probe passes when the job's dependencies are done.
    pub fn set_cutoff(&mut self, plan: CutoffPlan, fresh: Job) {
        self.cutoff = Some((plan, Box::new(fresh)));
    }

    /// justrust: the fresh alternative, if any, so callers can chain work
    /// onto it the same way they do for a fresh job.
    pub fn cutoff_job_mut(&mut self) -> Option<&mut Job> {
        self.cutoff.as_mut().map(|(_, job)| &mut **job)
    }

    /// justrust: called once all dependencies of this job finished. Returns
    /// the fresh alternative if every dependency's crate hash is unchanged,
    /// otherwise the job itself (which then runs rustc as usual).
    pub fn resolve_cutoff(mut self) -> Job {
        match self.cutoff.take() {
            Some((plan, fresh)) if plan.probe() => *fresh,
            _ => self,
        }
    }

    /// Consumes this job by running it, returning the result of the
    /// computation.
    pub fn run(self, state: &JobState<'_, '_>) -> CargoResult<()> {
        self.work.call(state)
    }

    /// Returns whether this job was fresh/dirty, where "fresh" means we're
    /// likely to perform just some small bookkeeping where "dirty" means we'll
    /// probably do something slow like invoke rustc.
    pub fn freshness(&self) -> &Freshness {
        &self.fresh
    }

    /// Chains the given work by putting it in front of our own unit of work.
    pub fn before(&mut self, next: Work) {
        let prev = mem::replace(&mut self.work, Work::noop());
        self.work = next.then(prev);
    }

    /// Chains the given work by putting it after of our own unit of work.
    pub fn after(&mut self, next: Work) {
        let prev = mem::replace(&mut self.work, Work::noop());
        self.work = prev.then(next);
    }
}

impl fmt::Debug for Job {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Job {{ ... }}")
    }
}

/// Indication of the freshness of a package.
///
/// A fresh package does not necessarily need to be rebuilt (unless a dependency
/// was also rebuilt), and a dirty package must always be rebuilt.
#[derive(Debug, Clone)]
pub enum Freshness {
    Fresh,
    Dirty(DirtyReason),
}

impl Freshness {
    pub fn is_dirty(&self) -> bool {
        matches!(self, Freshness::Dirty(_))
    }

    pub fn is_fresh(&self) -> bool {
        matches!(self, Freshness::Fresh)
    }
}
