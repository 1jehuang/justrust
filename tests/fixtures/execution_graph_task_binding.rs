#![feature(rustc_private)]
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_thread_pool;

use rustc_data_structures::sync::par_join;
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;
use rustc_thread_pool::execution_observer::{PoolPointUnbound, current_context};
use std::sync::{Arc, Mutex};

struct NestedQueries;
impl Callbacks for NestedQueries {
    fn after_expansion<'tcx>(&mut self, _: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let observer = tcx.sess.query_admission.as_ref();
        let pool = observer.map(|observer| observer.resource_pool().expect("owned frontend pool"));
        let entry = pool.as_ref().map(|pool| {
            let registry = rustc_thread_pool::Registry::current();
            assert!(Arc::ptr_eq(pool, registry.execution_observer().unwrap()));
            let task = current_context().expect("callback has an owned native frontend scope");
            assert!(pool.owns(&task));
            let point = pool.current_point().unwrap();
            let before = pool.snapshot();
            assert!(observer.unwrap().enter_frontend_scope().is_none());
            assert_eq!(pool.current_point().unwrap().task, point.task);
            let after = pool.snapshot();
            assert_eq!(
                before.events, after.events,
                "refused entry must not allocate a root"
            );
            assert_eq!(
                before.task_allocator_high_water,
                after.task_allocator_high_water
            );
            point.task
        });
        let owners: Vec<_> = tcx.hir_body_owners().collect();
        assert!(owners.len() >= 8, "fixture needs eight independent bodies");
        // Real compiler queries, with compiler-owned WorkerLocal/session globals.
        // Different bodies ensure the continuation does real, uncached work.
        par_join(
            || {
                par_join(
                    || {
                        tcx.typeck(owners[0]);
                    },
                    || {
                        tcx.typeck(owners[1]);
                    },
                );
                tcx.typeck(owners[2]);
            },
            || {
                par_join(
                    || {
                        tcx.typeck(owners[3]);
                    },
                    || {
                        tcx.typeck(owners[4]);
                    },
                );
                tcx.typeck(owners[5]);
            },
        );
        tcx.typeck(owners[6]);
        // Identify this real synchronous cached request and its dep read by
        // allocator range, not by finding an arbitrary bound request later.
        let cached_start = observer.map(|observer| observer.reads().high_waters());
        let continuation = pool.as_ref().map(|pool| pool.current_point().unwrap().task);
        tcx.typeck(owners[0]);
        let cached_end = observer.map(|observer| observer.reads().high_waters());
        tcx.typeck(owners[7]);

        // A generic dispatched job is not a child of the callback's segment.
        // Its cached request/read must stay unbound even though the helping
        // worker may have an owned frontend continuation beneath dispatch.
        let opaque_ranges = Mutex::new(None);
        let proof = rustc_data_structures::sync::check_dyn_thread_safe()
            .expect("fixture requires -Zthreads");
        let opaque = proof.derive(|| {
            if let Some(pool) = &pool {
                assert_eq!(pool.current_point(), Err(PoolPointUnbound::NoActiveTask));
                assert!(rustc_thread_pool::execution_observer::is_dispatch_masked());
                let before = pool.snapshot();
                assert!(observer.unwrap().enter_frontend_scope().is_none());
                let after = pool.snapshot();
                assert_eq!(before.events, after.events);
                assert_eq!(
                    before.task_allocator_high_water,
                    after.task_allocator_high_water
                );
                assert_eq!(pool.current_point(), Err(PoolPointUnbound::NoActiveTask));
            }
            let start = observer.map(|observer| observer.reads().high_waters());
            tcx.typeck(owners[0]);
            let end = observer.map(|observer| observer.reads().high_waters());
            if let (Some(start), Some(end)) = (start, end) {
                *opaque_ranges.lock().unwrap() = Some((start, end));
            }
            if let Some(pool) = &pool {
                assert_eq!(pool.current_point(), Err(PoolPointUnbound::NoActiveTask));
            }
        });
        rustc_thread_pool::scope(|scope| {
            scope.spawn(move |_| opaque.into_inner()());
        });
        if let Some(pool) = &pool {
            assert_eq!(pool.current_point().unwrap().task, continuation.unwrap());
            let (opaque_start, opaque_end) = opaque_ranges.into_inner().unwrap().unwrap();
            let start = cached_start.unwrap();
            let end = cached_end.unwrap();
            std::fs::write(
                std::env::var_os("JR_QUERY_SCOPE_WITNESS").expect("witness output path"),
                format!(
                    "{{\"entry_task\":{},\"continuation_task\":{},\"cached_requests\":[{},{}],\"cached_reads\":[{},{}],\"opaque_requests\":[{},{}],\"opaque_reads\":[{},{}]}}\n",
                    entry.unwrap(), continuation.unwrap(), start[0], end[0], start[1], end[1],
                    opaque_start[0], opaque_end[0], opaque_start[1], opaque_end[1],
                ),
            ).unwrap();
        }
        eprintln!(
            "native-task-binding-fixture: queried {} bodies",
            owners.len()
        );
        Compilation::Continue
    }
}
fn main() {
    let args: Vec<String> = std::env::args().collect();
    rustc_driver::compiler_entrypoint(&args, &mut NestedQueries);
}
