//! Explicit frontend admission adapter. Selection is a driver decision, never
//! an observer decision. Both callback pairs retain the same native proxy.
use std::sync::Arc;

use rustc_thread_pool::ThreadPoolBuilder;
use rustc_thread_pool::admission::{AdmissionAttachmentId, AdmissionContext};

use super::TicketedProxy;

type UnitHandler = Box<dyn Fn() + Send + Sync>;
type WaitHandler = Box<dyn Fn(AdmissionContext) + Send + Sync>;

/// One indivisible contextual pair and its independently minted identity.
struct WaitHandlers {
    attachment: AdmissionAttachmentId,
    acquire: WaitHandler,
    release: WaitHandler,
}

/// Native frontend administrative callbacks, optionally with contextual waits.
/// Missing diagnostic identity never disables the selected admission algorithm.
pub struct FrontendAdmission {
    acquire: UnitHandler,
    release: UnitHandler,
    waits: Option<WaitHandlers>,
}

impl FrontendAdmission {
    /// Wrap an existing proxy without changing its capture policy or algorithm.
    pub fn from_ticketed(proxy: Arc<TicketedProxy>) -> Self {
        Self::with_attachment(proxy, AdmissionAttachmentId::mint())
    }

    fn with_attachment(
        proxy: Arc<TicketedProxy>,
        attachment: Option<AdmissionAttachmentId>,
    ) -> Self {
        let acquire_proxy = Arc::clone(&proxy);
        let release_proxy = Arc::clone(&proxy);
        let waits = attachment.map(|attachment| {
            let acquire_proxy = Arc::clone(&proxy);
            let release_proxy = Arc::clone(&proxy);
            WaitHandlers {
                attachment,
                acquire: Box::new(move |_| acquire_proxy.acquire_thread()),
                release: Box::new(move |_| release_proxy.release_thread()),
            }
        });
        Self {
            acquire: Box::new(move || acquire_proxy.acquire_thread()),
            release: Box::new(move || release_proxy.release_thread()),
            waits,
        }
    }

    pub fn attachment_id(&self) -> Option<AdmissionAttachmentId> {
        self.waits.as_ref().map(|waits| waits.attachment)
    }

    /// Observe the existing administrative pair without selecting or replacing
    /// admission. Like contextual observers, these callbacks must be nonblocking
    /// and nonpanicking and must not allocate or affect admission decisions.
    pub fn with_unit_observer<A, R>(self, before_acquire: A, after_release: R) -> Self
    where
        A: Fn() + Send + Sync + 'static,
        R: Fn() + Send + Sync + 'static,
    {
        let Self { acquire, release, waits } = self;
        Self {
            acquire: Box::new(move || {
                before_acquire();
                acquire();
            }),
            release: Box::new(move || {
                release();
                after_release();
            }),
            waits,
        }
    }

    /// Observe only contextual waits, leaving administrative paths untouched.
    /// Observers run outside native proxy locks. They must not panic, block,
    /// allocate, or participate in admission decisions.
    pub fn with_wait_observer<A, R>(mut self, before_acquire: A, after_release: R) -> Self
    where
        A: Fn(AdmissionContext) + Send + Sync + 'static,
        R: Fn(AdmissionContext) + Send + Sync + 'static,
    {
        if let Some(waits) = self.waits.take() {
            let WaitHandlers { attachment, acquire, release } = waits;
            self.waits = Some(WaitHandlers {
                attachment,
                acquire: Box::new(move |context| {
                    before_acquire(context);
                    acquire(context);
                }),
                release: Box::new(move |context| {
                    release(context);
                    after_release(context);
                }),
            });
        }
        self
    }

    /// Install the grouped callbacks before pool workers are started.
    pub fn apply_to(self, builder: ThreadPoolBuilder) -> ThreadPoolBuilder {
        let builder =
            builder.acquire_thread_handler(self.acquire).release_thread_handler(self.release);
        match self.waits {
            Some(waits) => {
                builder.admission_handlers(waits.attachment, waits.acquire, waits.release)
            }
            None => builder,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rustc_thread_pool::admission::{AdmissionPhase, QualifiedWaitId, QueryRootNamespace};

    use super::*;

    #[test]
    fn capture_and_attachment_do_not_select_or_disable_ticketed_admission() {
        super::super::initialize(1, |_| {});
        for capacity in [0, 64] {
            for attachment in [None, AdmissionAttachmentId::mint()] {
                let proxy = TicketedProxy::new_with_event_capacity(capacity);
                let admission = FrontendAdmission::with_attachment(Arc::clone(&proxy), attachment);
                assert_eq!(admission.attachment_id(), attachment);
                (admission.release)();
                (admission.acquire)();
                (admission.release)();
                let snapshot = proxy.debug_snapshot();
                assert_eq!((snapshot.used, snapshot.pending), (0, 0));
                assert_eq!(snapshot.inventory.requests, 1);
                assert_eq!(snapshot.inventory.grants, 1);
                assert_eq!(snapshot.inventory.acks, 1);
                assert_eq!(snapshot.inventory.releases, 2);
                assert_eq!(snapshot.inventory.lost_events, 0);
                assert_eq!(snapshot.events.is_empty(), capacity == 0);
                assert_eq!(admission.waits.is_some(), attachment.is_some());
            }
        }
    }

    #[test]
    fn grouped_observers_preserve_identity_and_do_not_wrap_administrative_paths() {
        super::super::initialize(1, |_| {});
        let proxy = TicketedProxy::new_with_event_capacity(0);
        let admission = FrontendAdmission::from_ticketed(Arc::clone(&proxy));
        let attachment_id = admission.attachment_id().unwrap();
        let observations = Arc::new(AtomicUsize::new(0));
        let acquired = Arc::clone(&observations);
        let released = Arc::clone(&observations);
        let admission = admission.with_wait_observer(
            move |_| {
                acquired.fetch_add(1, Ordering::Relaxed);
            },
            move |_| {
                released.fetch_add(1, Ordering::Relaxed);
            },
        );
        assert_eq!(admission.attachment_id(), Some(attachment_id));
        (admission.release)();
        (admission.acquire)();
        assert_eq!(observations.load(Ordering::Relaxed), 0);
        let context = AdmissionContext {
            attachment_id,
            qualified_wait_id: QualifiedWaitId {
                query_root: QueryRootNamespace::mint().unwrap(),
                wait_id: NonZeroU64::new(1).unwrap(),
            },
            phase: AdmissionPhase::QueryWaitRelease,
        };
        let waits = admission.waits.as_ref().unwrap();
        (waits.release)(context);
        assert_eq!(proxy.debug_snapshot().used, 0);
        (waits.acquire)(AdmissionContext { phase: AdmissionPhase::QueryWaitReacquire, ..context });
        assert_eq!(proxy.debug_snapshot().used, 1);
        assert_eq!(observations.load(Ordering::Relaxed), 2);
        (admission.release)();
        let snapshot = proxy.debug_snapshot();
        assert_eq!(snapshot.inventory.requests, 2);
        assert_eq!(snapshot.inventory.acks, 2);
        assert!(snapshot.events.is_empty());
    }

    #[test]
    fn unit_observers_preserve_optional_attachment_and_selected_inventory() {
        super::super::initialize(1, |_| {});
        for attachment in [None, AdmissionAttachmentId::mint()] {
            let proxy = TicketedProxy::new_with_event_capacity(0);
            let acquired = Arc::new(AtomicUsize::new(0));
            let released = Arc::new(AtomicUsize::new(0));
            let acquired_observer = Arc::clone(&acquired);
            let released_observer = Arc::clone(&released);
            let admission = FrontendAdmission::with_attachment(Arc::clone(&proxy), attachment)
                .with_unit_observer(
                    move || {
                        acquired_observer.fetch_add(1, Ordering::Relaxed);
                    },
                    move || {
                        released_observer.fetch_add(1, Ordering::Relaxed);
                    },
                );
            assert_eq!(admission.attachment_id(), attachment);
            (admission.release)();
            (admission.acquire)();
            (admission.release)();
            assert_eq!(acquired.load(Ordering::Relaxed), 1);
            assert_eq!(released.load(Ordering::Relaxed), 2);
            let snapshot = proxy.debug_snapshot();
            assert_eq!((snapshot.used, snapshot.pending), (0, 0));
            assert_eq!(snapshot.inventory.requests, 1);
            assert_eq!(snapshot.inventory.acks, 1);
            assert!(snapshot.events.is_empty());
        }
    }
}
