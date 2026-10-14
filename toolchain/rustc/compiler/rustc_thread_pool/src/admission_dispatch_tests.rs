//! Focused tests of the real registry's contextual admission path.
use std::num::NonZeroU64;
use std::sync::{Arc, Mutex};

use crate::admission::*;
use crate::registry::{Registry, WorkerThread};
use crate::{
    ThreadPoolBuilder, mark_blocked_and_wait, mark_blocked_and_wait_with_context, mark_unblocked,
};

type Events = Arc<Mutex<Vec<&'static str>>>;

fn binding() -> AdmissionBinding {
    AdmissionBinding {
        attachment_id: AdmissionAttachmentId::mint().unwrap(),
        query_root: QueryRootNamespace::mint().unwrap(),
    }
}

fn context(binding: AdmissionBinding, phase: AdmissionPhase) -> AdmissionContext {
    AdmissionContext {
        attachment_id: binding.attachment_id,
        qualified_wait_id: QualifiedWaitId {
            query_root: binding.query_root,
            wait_id: NonZeroU64::new(1).unwrap(),
        },
        phase,
    }
}

fn record(events: &Events, name: &'static str) -> impl Fn() + Send + Sync + 'static {
    let events = events.clone();
    move || events.lock().unwrap().push(name)
}

#[test]
fn paired_legacy_admin_foreign_and_phase_dispatch_once() {
    for paired in [false, true] {
        let events = Events::default();
        let binding = binding();
        let mut builder = ThreadPoolBuilder::new()
            .num_threads(1)
            .acquire_thread_handler(record(&events, "unit acquire"))
            .release_thread_handler(record(&events, "unit release"))
            .spawn_handler(|_| Ok(()));
        if paired {
            let acquire = record(&events, "context acquire");
            let release = record(&events, "context release");
            builder = builder.deadlock_handler(|| {}).admission_handlers(
                binding.attachment_id,
                move |_| acquire(),
                move |_| release(),
            );
        }
        let registry = Registry::new(builder).unwrap();
        registry.acquire_thread();
        registry.release_thread();
        registry.acquire_thread_with_context(context(binding, AdmissionPhase::QueryWaitReacquire));
        registry.release_thread_with_context(context(binding, AdmissionPhase::QueryWaitRelease));
        let foreign =
            AdmissionBinding { attachment_id: AdmissionAttachmentId::mint().unwrap(), ..binding };
        registry.acquire_thread_with_context(context(foreign, AdmissionPhase::QueryWaitReacquire));
        registry.release_thread_with_context(context(foreign, AdmissionPhase::QueryWaitRelease));
        registry.acquire_thread_with_context(context(binding, AdmissionPhase::QueryWaitRelease));
        registry.release_thread_with_context(context(binding, AdmissionPhase::QueryWaitReacquire));
        assert_eq!(
            *events.lock().unwrap(),
            [
                "unit acquire",
                "unit release",
                if paired { "context acquire" } else { "unit acquire" },
                if paired { "context release" } else { "unit release" },
                "unit acquire",
                "unit release",
                "unit acquire",
                "unit release",
            ]
        );
        registry.terminate();
    }
}

#[test]
fn absent_adapters_are_noops() {
    let builder = ThreadPoolBuilder::new().num_threads(1).spawn_handler(|_| Ok(()));
    let registry = Registry::new(builder).unwrap();
    let binding = binding();
    registry.acquire_thread_with_context(context(binding, AdmissionPhase::QueryWaitReacquire));
    registry.release_thread_with_context(context(binding, AdmissionPhase::QueryWaitRelease));
    registry.terminate();
}

struct Guard(Events);
impl Drop for Guard {
    fn drop(&mut self) {
        self.0.lock().unwrap().push("guard dropped");
    }
}

#[test]
fn contextual_wait_drops_guard_before_reacquire_and_return_or_unwind() {
    for panic in [false, true] {
        let events = Events::default();
        let binding = binding();
        let acquire = record(&events, "reacquired");
        let release = record(&events, "released");
        let pool = ThreadPoolBuilder::new()
            .num_threads(1)
            .deadlock_handler(|| {})
            .admission_handlers(binding.attachment_id, move |_| acquire(), move |_| release())
            .build()
            .unwrap();
        pool.install(|| {
            let guard = Guard(events.clone());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let events = events.clone();
                mark_blocked_and_wait_with_context(
                    binding,
                    NonZeroU64::new(1).unwrap(),
                    move || {
                        let _guard = guard;
                        unsafe {
                            mark_unblocked((*WorkerThread::current()).registry());
                        }
                        assert!(!events.lock().unwrap().contains(&"reacquired"));
                        if panic {
                            panic!("wait panic");
                        }
                    },
                );
            }));
            assert_eq!(result.is_err(), panic);
        });
        assert_eq!(*events.lock().unwrap(), ["released", "guard dropped", "reacquired"]);
    }
}

#[test]
fn legacy_wait_does_not_invoke_context_pair() {
    let binding = binding();
    let pool = ThreadPoolBuilder::new()
        .num_threads(1)
        .deadlock_handler(|| {})
        .admission_handlers(
            binding.attachment_id,
            |_| panic!("context acquire"),
            |_| panic!("context release"),
        )
        .build()
        .unwrap();
    pool.install(|| {
        mark_blocked_and_wait(|| unsafe {
            mark_unblocked((*WorkerThread::current()).registry());
        })
    });
}
