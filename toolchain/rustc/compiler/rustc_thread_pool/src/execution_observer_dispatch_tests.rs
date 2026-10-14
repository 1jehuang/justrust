//! Real-pool regressions for opaque helper dispatch, not inferred scheduler edges.

use std::sync::{Arc, Barrier, Mutex};

use crate::execution_observer::{Event, ExecutionObserver, current_task};

#[test]
fn opaque_scope_help_does_not_inherit_running_segment() {
    let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let observer = ExecutionObserver::new(100).unwrap();
    let observations = Mutex::new(None);
    pool.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
        let root = observer.root().unwrap();
        let root_id = root.id();
        let guard = root.enter().unwrap();
        crate::scope(|scope| {
            scope.spawn(|_| {
                let entry = current_task();
                let children = crate::join(
                    || current_task().unwrap(),
                    || current_task().unwrap(),
                );
                let after_join = current_task();
                *observations.lock().unwrap() = Some((entry, children, after_join));
            });
            assert_eq!(crate::yield_local(), Some(crate::Yield::Executed));
        });
        let restored = current_task();
        let (entry, children, after_join) = observations.lock().unwrap().unwrap();
        let receipt = observer.snapshot();
        eprintln!(
            "opaque helper witness: root={root_id:?}, entry={entry:?}, children={children:?}, after_join={after_join:?}, restored={restored:?}, events={:?}",
            receipt.events
        );
        assert_eq!(entry, None, "opaque scope job inherited the helping task");
        assert_eq!(after_join, None, "opaque job retained a nested join continuation");
        assert_ne!(children.0, children.1);
        assert_eq!(restored, Some(root_id), "helper dispatch mutated the suspended origin");
        assert!(receipt.events.iter().any(|event| matches!(event,
            Event::Root { task } if *task != root_id)));
        assert!(!receipt.events.iter().any(|event| matches!(event,
            Event::Fork { parent, .. } if *parent == root_id)),
            "join in opaque work was falsely parented to the helping task");
        drop(guard);
        assert_eq!(current_task(), None);
    });
}

#[test]
fn opaque_scope_nested_help_restores_the_instrumented_child() {
    let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let observer = ExecutionObserver::new(100).unwrap();
    let observations = Mutex::new(None);
    pool.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
        crate::join(
            || {
                let child = current_task().unwrap();
                crate::scope(|scope| {
                    scope.spawn(|_| {
                        let outer_entry = current_task();
                        crate::scope(|inner| {
                            inner.spawn(|_| {
                                *observations.lock().unwrap() = Some((outer_entry, current_task()));
                            });
                            assert_eq!(crate::yield_local(), Some(crate::Yield::Executed));
                        });
                        assert_eq!(current_task(), outer_entry);
                    });
                    assert_eq!(crate::yield_local(), Some(crate::Yield::Executed));
                });
                let restored = current_task();
                let observed = observations.lock().unwrap().unwrap();
                eprintln!("nested helper witness: child={child:?}, entries={observed:?}, restored={restored:?}");
                assert_eq!(observed, (None, None), "opaque nested jobs inherited the child");
                assert_eq!(restored, Some(child));
            },
            || assert!(current_task().is_some()),
        );
        assert_eq!(current_task(), None);
    });
}

#[test]
fn opaque_scope_panic_restores_the_running_segment() {
    let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let observer = ExecutionObserver::new(100).unwrap();
    let observations = Mutex::new(None);
    pool.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
        let root = observer.root().unwrap();
        let root_id = root.id();
        let guard = root.enter().unwrap();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::scope(|scope| {
                scope.spawn(|_| {
                    *observations.lock().unwrap() = Some(current_task());
                    std::panic::panic_any("opaque helper panic");
                });
                assert_eq!(crate::yield_local(), Some(crate::Yield::Executed));
            });
        }));
        let payload = result.expect_err("opaque panic must propagate");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"opaque helper panic"));
        let entry = observations.lock().unwrap().unwrap();
        let restored = current_task();
        eprintln!("panic helper witness: root={root_id:?}, entry={entry:?}, restored={restored:?}");
        assert_eq!(entry, None, "panicking opaque job inherited the helping task");
        assert_eq!(restored, Some(root_id));
        drop(guard);
        assert_eq!(current_task(), None);
    });
}

#[test]
fn opaque_fifo_help_does_not_inherit_running_segment() {
    let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let observer = ExecutionObserver::new(100).unwrap();
    let observations = Mutex::new(None);
    pool.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
        let root = observer.root().unwrap();
        let guard = root.enter().unwrap();
        crate::scope_fifo(|scope| {
            scope.spawn_fifo(|_| *observations.lock().unwrap() = Some(current_task()));
            assert_eq!(crate::yield_local(), Some(crate::Yield::Executed));
        });
        assert_eq!(observations.lock().unwrap().unwrap(), None);
        assert_eq!(current_task(), Some(root.id()));
        drop(guard);
    });
}

#[test]
fn instrumented_stolen_children_reenter_after_dispatch_mask() {
    let pool = crate::ThreadPoolBuilder::new().num_threads(2).build().unwrap();
    let observer = ExecutionObserver::new(100).unwrap();
    let barrier = Barrier::new(2);
    pool.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
        let (inline, stolen) = crate::join_context(
            |context| {
                barrier.wait();
                (context.migrated(), current_task().unwrap())
            },
            |context| {
                barrier.wait();
                (context.migrated(), current_task().unwrap())
            },
        );
        assert!(!inline.0);
        assert!(stolen.0, "the barrier requires a genuinely stolen sibling");
        assert_ne!(inline.1, stolen.1);
        assert_eq!(current_task(), None);
    });
}

#[test]
fn dispatch_mask_restores_exact_suspended_fork_slot() {
    let observer = ExecutionObserver::new(100).unwrap();
    let root = observer.root().unwrap();
    let guard = root.enter().unwrap();
    let fork = root.fork(2).unwrap();
    {
        let _mask = crate::execution_observer::suspend_for_dispatch();
        assert_eq!(current_task(), None);
        {
            let _nested_mask = crate::execution_observer::suspend_for_dispatch();
            assert_eq!(current_task(), None);
        }
    }
    for child in fork.children() {
        let _child_guard = child.enter().unwrap();
    }
    let continuation = fork.resume().ok().expect("exact original Active slot must be restored");
    assert_eq!(current_task(), Some(continuation));
    drop(guard);
    assert_eq!(current_task(), None);
}

#[test]
fn dispatch_mask_unwind_restores_without_lifecycle_events() {
    let observer = ExecutionObserver::new(100).unwrap();
    let root = observer.root().unwrap();
    let guard = root.enter().unwrap();
    let before = observer.snapshot().events;
    let result = std::panic::catch_unwind(|| {
        let _mask = crate::execution_observer::suspend_for_dispatch();
        assert_eq!(current_task(), None);
        std::panic::panic_any("dispatch unwind");
    });
    let payload = result.expect_err("dispatch unwind must propagate");
    assert_eq!(payload.downcast_ref::<&str>(), Some(&"dispatch unwind"));
    assert_eq!(current_task(), Some(root.id()));
    assert_eq!(observer.snapshot().events, before);
    drop(guard);
    assert_eq!(current_task(), None);
}

#[test]
fn opaque_scope_completion_help_is_unbound() {
    let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let observer = ExecutionObserver::new(100).unwrap();
    let observations = Mutex::new(None);
    pool.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
        let root = observer.root().unwrap();
        let guard = root.enter().unwrap();
        crate::scope(|scope| {
            scope.spawn(|_| *observations.lock().unwrap() = Some(current_task()));
        });
        assert_eq!(observations.lock().unwrap().unwrap(), None);
        assert_eq!(current_task(), Some(root.id()));
        drop(guard);
    });
}

#[test]
fn opaque_broadcast_help_is_unbound() {
    let pool = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let observer = ExecutionObserver::new(100).unwrap();
    pool.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&observer)));
        let root = observer.root().unwrap();
        let guard = root.enter().unwrap();
        assert_eq!(crate::broadcast(|_| current_task()), vec![None]);
        assert_eq!(current_task(), Some(root.id()));
        drop(guard);
    });
}

#[test]
fn cross_registry_install_preserves_source_and_destination_contexts() {
    let source = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let destination = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let unobserved = crate::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
    let source_observer = ExecutionObserver::new(100).unwrap();
    let destination_observer = ExecutionObserver::new(100).unwrap();
    source.install(|| {
        assert!(crate::Registry::current().attach_execution_observer(Arc::clone(&source_observer)));
        let root = source_observer.root().unwrap();
        let guard = root.enter().unwrap();
        destination.install(|| {
            assert_eq!(current_task(), None);
            assert!(
                crate::Registry::current()
                    .attach_execution_observer(Arc::clone(&destination_observer))
            );
            let (first, second) =
                crate::join(|| current_task().unwrap(), || current_task().unwrap());
            assert_ne!(first, second);
            assert_ne!(first.observer, root.id().observer);
            assert_eq!(current_task(), None);
        });
        assert_eq!(current_task(), Some(root.id()));
        unobserved.install(|| {
            assert_eq!(current_task(), None);
            assert_eq!(crate::join(current_task, current_task), (None, None));
        });
        assert_eq!(current_task(), Some(root.id()));
        drop(guard);
        assert_eq!(current_task(), None);
    });
}
