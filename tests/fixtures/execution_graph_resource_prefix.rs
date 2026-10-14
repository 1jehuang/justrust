#![feature(rustc_private)]
//! Actual native resource-prefix exercise, separate from accepted selected-six.
//! Full native journaling is prepared OFF and ON. OFF suppresses publication,
//! not journal overhead. Native owners alone publish the triple endorsements.
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_thread_pool;

use rustc_data_structures::jobserver::{
    self, Acquired, CaptureCutReason, FrontendAdmission, ResourceCaptureConfig,
    ResourceCaptureControl, TicketedProxy,
};
use rustc_data_structures::sync::par_join;
use rustc_driver::{Callbacks, Compilation};
use rustc_hir::def_id::LocalDefId;
use rustc_interface::interface::{Compiler, Config};
use rustc_middle::ty::{TyCtxt, TypeckResults};
use rustc_session::query_admission::QueryAdmissionObserver;
use rustc_thread_pool::{AdmissionPhase, QualifiedWaitId};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

const LOCAL_TRANSFER: bool = cfg!(resource_local_transfer);
const PREGRANT_CUT: bool = cfg!(resource_cut_pregrant);
#[cfg(resource_cap_zero)]
const TRANSACTION_CAPACITY: usize = 0;
#[cfg(all(not(resource_cap_zero), resource_cap_tiny))]
const TRANSACTION_CAPACITY: usize = 1;
#[cfg(all(not(resource_cap_zero), not(resource_cap_tiny)))]
const TRANSACTION_CAPACITY: usize = 4096;

thread_local! {
    static DUPLICATE: Cell<bool> = const { Cell::new(false) };
    static QUALIFIED_RELEASE: Cell<Option<QualifiedWaitId>> = const { Cell::new(None) };
}
static SURPLUS: Mutex<Option<Vec<Acquired>>> = Mutex::new(None);
static PROXY: OnceLock<Arc<TicketedProxy>> = OnceLock::new();
static CAPTURE: OnceLock<ResourceCaptureControl> = OnceLock::new();
type Typeck = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx TypeckResults<'tcx>;
static TYPECK: OnceLock<Typeck> = OnceLock::new();
static READY: AtomicBool = AtomicBool::new(false);
static ARMED: AtomicBool = AtomicBool::new(false);
static ENTERED: AtomicBool = AtomicBool::new(false);
static FINISH_QUERY: AtomicBool = AtomicBool::new(false);
static PRODUCER_PARKED: AtomicBool = AtomicBool::new(false);
static FINISH_PRODUCER: AtomicBool = AtomicBool::new(false);
static WAITER_RETURNED: AtomicBool = AtomicBool::new(false);
static FINISH_WAITER: AtomicBool = AtomicBool::new(false);
static UNIT_RELEASES: AtomicU64 = AtomicU64::new(0);
static UNIT_ACQUIRES: AtomicU64 = AtomicU64::new(0);
static CONTEXT_RELEASES: AtomicU64 = AtomicU64::new(0);
static CONTEXT_ACQUIRES: AtomicU64 = AtomicU64::new(0);
static WAIT_ID: AtomicU64 = AtomicU64::new(0);
static INVALID_CONTEXT: AtomicBool = AtomicBool::new(false);
static STOP_WATCHDOG: AtomicBool = AtomicBool::new(false);

fn load(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}
fn set(flag: &AtomicBool) {
    flag.store(true, Ordering::SeqCst);
}
fn wait(label: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(25);
    while !ready() {
        assert!(Instant::now() < deadline, "insufficientexercise: {label}");
        std::thread::yield_now();
    }
}
fn typeck<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> &'tcx TypeckResults<'tcx> {
    if ARMED.swap(false, Ordering::SeqCst) {
        set(&ENTERED);
        // Provider execution is outside query cache locks. Publication remains
        // the query engine's operation, not a model or fixture-written result.
        wait("controller did not permit provider completion", || {
            load(&FINISH_QUERY)
        });
    }
    TYPECK.get().unwrap()(tcx, key)
}
struct ResourcePrefix {
    observer: Option<QueryAdmissionObserver>,
}
impl Callbacks for ResourcePrefix {
    fn config(&mut self, config: &mut Config) {
        config.query_admission = self.observer.take();
        config.make_frontend_admission = Some(Box::new(|| {
            // Real process supply may exceed frontend worker count. Reserve it
            // BEFORE proxy/helper/workers, not by faking a capacity-two Client.
            let client = jobserver::client();
            let available = client.available().expect("native supply inventory");
            assert!(
                available >= 1,
                "insufficientexercise: no second-worker supply"
            );
            let mut surplus = Vec::with_capacity(available - 1);
            for _ in 1..available {
                surplus.push(client.acquire().expect("real surplus acquisition"));
            }
            assert_eq!(client.available().unwrap(), 1);
            *SURPLUS.lock().unwrap() = Some(surplus);
            let proxy = TicketedProxy::new_with_event_capacity(0);
            let capture = proxy
                .prepare_resource_capture(ResourceCaptureConfig::new(TRANSACTION_CAPACITY))
                .expect("resource capture preparation");
            assert!(PROXY.set(Arc::clone(&proxy)).is_ok());
            assert!(CAPTURE.set(capture.clone()).is_ok());
            // Production adapter/interface register and publish actual query,
            // pool and resource owners. Fixture never writes positive docs.
            let admission = FrontendAdmission::from_ticketed_with_resource(proxy, capture)
                .with_unit_observer(
                    || {
                        if DUPLICATE.with(Cell::get) {
                            UNIT_ACQUIRES.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                    || {
                        if DUPLICATE.with(Cell::get) {
                            UNIT_RELEASES.fetch_add(1, Ordering::SeqCst);
                        }
                    },
                );
            let expected = admission.attachment_id().expect("attachment exhausted");
            admission.with_wait_observer(
                move |context| {
                    if !DUPLICATE.with(Cell::get) {
                        return;
                    }
                    if context.attachment_id != expected
                        || context.phase != AdmissionPhase::QueryWaitReacquire
                        || QUALIFIED_RELEASE.with(Cell::get) != Some(context.qualified_wait_id)
                    {
                        INVALID_CONTEXT.store(true, Ordering::SeqCst);
                    }
                    WAIT_ID.store(context.qualified_wait_id.wait_id.get(), Ordering::SeqCst);
                    CONTEXT_ACQUIRES.fetch_add(1, Ordering::SeqCst);
                },
                move |context| {
                    if !DUPLICATE.with(Cell::get) {
                        return;
                    }
                    if context.attachment_id != expected
                        || context.phase != AdmissionPhase::QueryWaitRelease
                    {
                        INVALID_CONTEXT.store(true, Ordering::SeqCst);
                    }
                    QUALIFIED_RELEASE
                        .with(|released| released.set(Some(context.qualified_wait_id)));
                    CONTEXT_RELEASES.fetch_add(1, Ordering::SeqCst);
                },
            )
        }));
        config.override_queries = Some(|_, providers| {
            assert!(TYPECK.set(providers.queries.typeck_root).is_ok());
            providers.queries.typeck_root = typeck;
        });
    }
    fn after_expansion<'tcx>(&mut self, _: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
        let owner = tcx
            .hir_body_owners()
            .find(|id| tcx.item_name(id.to_def_id()).as_str() == "resource_target")
            .expect("fixture missing resource_target");
        let proxy = Arc::clone(PROXY.get().expect("explicit factory not called"));
        let capture = CAPTURE.get().unwrap().clone();
        let client = jobserver::client();
        let qualified = tcx.sess.query_admission.is_some();
        let surplus = SURPLUS
            .lock()
            .unwrap()
            .take()
            .expect("factory surplus ownership");
        set(&ARMED);
        let controller = std::thread::spawn(move || {
            // All snapshots and cut requests are exclusively on this controller.
            let baseline_releases = proxy.debug_snapshot().inventory.releases;
            set(&READY);
            wait("actual duplicate query release absent", || {
                UNIT_RELEASES.load(Ordering::SeqCst) + CONTEXT_RELEASES.load(Ordering::SeqCst) > 0
            });
            let released = proxy.debug_snapshot();
            assert!(released.inventory.releases > baseline_releases);
            assert_eq!((released.used, released.pending), (1, 0));
            let held = client.acquire().expect("controller real released supply");
            assert_eq!(client.available().unwrap(), 0);
            set(&FINISH_QUERY);
            wait("producer query did not finish", || load(&PRODUCER_PARKED));
            wait("actual query-ready reacquire absent", || {
                UNIT_ACQUIRES.load(Ordering::SeqCst) + CONTEXT_ACQUIRES.load(Ordering::SeqCst) > 0
            });
            wait("actual native Pending absent", || {
                proxy.debug_snapshot().pending > 0
            });
            let pending = proxy.debug_snapshot();
            assert_eq!((pending.used, pending.pending), (1, 1));
            assert!(
                !load(&WAITER_RETURNED),
                "query result alone authorized native return"
            );
            assert!(!load(&INVALID_CONTEXT));
            assert_eq!(CONTEXT_ACQUIRES.load(Ordering::SeqCst) > 0, qualified);
            if PREGRANT_CUT {
                assert!(capture.seal_prefix(CaptureCutReason::ControllerCheckpoint));
            }
            // Exactly one real source is enabled. Never inject proxy.release or
            // release_raw. Local source must be the actual parent join release.
            let held = if LOCAL_TRANSFER {
                set(&FINISH_PRODUCER);
                Some(held)
            } else {
                drop(held);
                None
            };
            wait("actual native grant did not resume query", || {
                load(&WAITER_RETURNED)
            });
            let resumed = proxy.debug_snapshot();
            assert!(resumed.inventory.grants > pending.inventory.grants);
            assert!(resumed.inventory.acks > pending.inventory.acks);
            if !PREGRANT_CUT {
                assert!(capture.seal_prefix(CaptureCutReason::ControllerCheckpoint));
            }
            assert!(
                !capture.seal_prefix(CaptureCutReason::ControllerCheckpoint),
                "prefix recut"
            );
            // checkpoint() is FROZEN after seal. Authoritative retained rows,
            // independent losses and source/operation IDs come from the native
            // finalizer receipt, never this diagnostic snapshot or TLS facts.
            let frozen = capture.checkpoint();
            eprintln!(
                "resource-prefix-evidence: {{\"source\":\"{}\",\"cut\":\"{}\",\"transaction_capacity\":{},\"reserved_real_surplus\":{},\"actual_query_release\":true,\"producer_completed\":true,\"native_pending\":true,\"returned_before_source\":false,\"returned_after_source\":true,\"qualified\":{},\"wait_id\":{},\"frozen_requests\":{},\"frozen_acks\":{},\"frozen_returns\":{},\"publication_only_off\":true}}",
                if LOCAL_TRANSFER { "local" } else { "helper" },
                if PREGRANT_CUT {
                    "pregrant"
                } else {
                    "postresume"
                },
                TRANSACTION_CAPACITY,
                surplus.len(),
                qualified,
                WAIT_ID.load(Ordering::SeqCst),
                frozen.counters.requests,
                frozen.counters.acks,
                frozen.counters.acquire_returns,
            );
            // No take/export here: actor joins are NOT frontend scope teardown.
            // Production outer scope guard takes frozen facts after workers end.
            set(&FINISH_PRODUCER);
            set(&FINISH_WAITER);
            (surplus, held)
        });
        wait("controller did not initialize", || load(&READY));
        par_join(
            || {
                tcx.typeck(owner);
                set(&PRODUCER_PARKED);
                wait("producer outer closure not released", || {
                    load(&FINISH_PRODUCER)
                });
            },
            || {
                wait("producer override not entered", || load(&ENTERED));
                DUPLICATE.with(|active| active.set(true));
                tcx.typeck(owner);
                DUPLICATE.with(|active| active.set(false));
                // Native adapter AcquireReturned and query token resumed have
                // already happened. Park ONLY this outer closure outside locks.
                set(&WAITER_RETURNED);
                wait("waiter outer closure not released", || load(&FINISH_WAITER));
            },
        );
        let (surplus, held) = controller.join().expect("resource controller failure");
        drop(held);
        drop(surplus);
        Compilation::Continue
    }
}
fn main() {
    // Existing transport is observation only; selection is always factory code.
    let observer = unsafe {
        rustc_session::query_binding::admit_inherited(
            rustc_session::query_binding::protocol::RESERVED_TICKET_FD,
        )
    };
    let watchdog = std::thread::spawn(|| {
        let deadline = Instant::now() + Duration::from_secs(100);
        while !load(&STOP_WATCHDOG) {
            if Instant::now() >= deadline {
                eprintln!("insufficientexercise: resource-prefix native fixture deadline");
                std::process::exit(86);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let args: Vec<String> = std::env::args().collect();
    rustc_driver::compiler_entrypoint(&args, &mut ResourcePrefix { observer });
    set(&STOP_WATCHDOG);
    watchdog.join().unwrap();
}
