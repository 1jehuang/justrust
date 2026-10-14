//! Opaque, process-unique identities for contextual admission callbacks.

use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_ATTACHMENT: AtomicU64 = AtomicU64::new(1);
static NEXT_QUERY_ROOT: AtomicU64 = AtomicU64::new(1);

// Zero is the permanently exhausted state. MAX is issued exactly once.
fn mint(counter: &AtomicU64) -> Option<NonZeroU64> {
    let mut next = counter.load(Ordering::Relaxed);
    loop {
        let id = NonZeroU64::new(next)?;
        match counter.compare_exchange_weak(
            next,
            next.checked_add(1).unwrap_or(0),
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Some(id),
            Err(actual) => next = actual,
        }
    }
}

/// An opaque identity for one admission-controller attachment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AdmissionAttachmentId(NonZeroU64);

impl AdmissionAttachmentId {
    /// Mint a never-reused identity, or return `None` after exhaustion.
    pub fn mint() -> Option<Self> {
        mint(&NEXT_ATTACHMENT).map(Self)
    }
}

/// An opaque namespace for the wait IDs of one query root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QueryRootNamespace(NonZeroU64);

impl QueryRootNamespace {
    /// Mint a never-reused namespace, or return `None` after exhaustion.
    pub fn mint() -> Option<Self> {
        mint(&NEXT_QUERY_ROOT).map(Self)
    }
}

/// A wait identity qualified by its query root.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct QualifiedWaitId {
    pub query_root: QueryRootNamespace,
    pub wait_id: NonZeroU64,
}

/// The query-wait transition represented by a callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionPhase {
    QueryWaitRelease,
    QueryWaitReacquire,
}

/// Context supplied to an admission callback.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionContext {
    pub attachment_id: AdmissionAttachmentId,
    pub qualified_wait_id: QualifiedWaitId,
    pub phase: AdmissionPhase,
}

/// The controller attachment and query root associated with a query.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdmissionBinding {
    pub attachment_id: AdmissionAttachmentId,
    pub query_root: QueryRootNamespace,
}

pub(crate) struct AdmissionHandlers {
    pub(crate) expected_attachment: AdmissionAttachmentId,
    pub(crate) acquire: Box<dyn Fn(AdmissionContext) + Send + Sync>,
    pub(crate) release: Box<dyn Fn(AdmissionContext) + Send + Sync>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injected_counter_exhaustion_is_permanent() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(mint(&counter).unwrap().get(), u64::MAX - 1);
        assert_eq!(mint(&counter).unwrap().get(), u64::MAX);
        assert_eq!(mint(&counter), None);
        assert_eq!(mint(&counter), None);
    }

    #[test]
    fn injected_counter_concurrent_exhaustion_never_reuses_ids() {
        let counter = AtomicU64::new(u64::MAX - 3);
        let ids = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..8).map(|_| scope.spawn(|| mint(&counter))).collect();
            handles.into_iter().filter_map(|handle| handle.join().unwrap()).collect::<Vec<_>>()
        });
        let mut unique = ids.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(ids.len(), 4);
        assert_eq!(unique.len(), 4);
        assert_eq!(mint(&counter), None);
        let independent = AtomicU64::new(1);
        assert_eq!(mint(&independent).unwrap().get(), 1);
    }

    #[test]
    fn namespaces_prevent_stale_wait_aliases() {
        let counter = AtomicU64::new(1);
        let first = QueryRootNamespace(mint(&counter).unwrap());
        let second = QueryRootNamespace(mint(&counter).unwrap());
        let wait_id = NonZeroU64::new(1).unwrap();
        assert_ne!(
            QualifiedWaitId { query_root: first, wait_id },
            QualifiedWaitId { query_root: second, wait_id }
        );
    }
}
