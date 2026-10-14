#![feature(rustc_private)]
//! Real selected frontend admission. No modeled semaphore and no policy env/argv.
//! Only the controller snapshots. Callbacks publish preallocated atomic facts.
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_hir;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;
extern crate rustc_thread_pool;

use rustc_data_structures::jobserver::{self, Acquired, FrontendAdmission, TicketedProxy};
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

thread_local! {
    static DUPLICATE: Cell<bool> = const { Cell::new(false) };
    static QUALIFIED_RELEASE: Cell<Option<QualifiedWaitId>> = const { Cell::new(None) };
}
static SURPLUS: Mutex<Option<Vec<Acquired>>> = Mutex::new(None);
static CONTROLLER_READY: AtomicBool = AtomicBool::new(false);
static UNIT_RELEASES: AtomicU64 = AtomicU64::new(0);
static UNIT_REACQUIRES: AtomicU64 = AtomicU64::new(0);
static PROXY: OnceLock<Arc<TicketedProxy>> = OnceLock::new();
type Typeck = for<'tcx> fn(TyCtxt<'tcx>, LocalDefId) -> &'tcx TypeckResults<'tcx>;
static TYPECK: OnceLock<Typeck> = OnceLock::new();
static FACTS: Facts = Facts {
    armed: AtomicBool::new(false),
    producer_entered: AtomicBool::new(false),
    finish_query: AtomicBool::new(false),
    producer_parked: AtomicBool::new(false),
    unpark_producer: AtomicBool::new(false),
    waiter_returned: AtomicBool::new(false),
    query_releases: AtomicU64::new(0),
    query_reacquires: AtomicU64::new(0),
    release_wait: AtomicU64::new(0),
    reacquire_wait: AtomicU64::new(0),
    invalid_context: AtomicBool::new(false),
    stop_watchdog: AtomicBool::new(false),
};
struct Facts {
    armed: AtomicBool,
    producer_entered: AtomicBool,
    finish_query: AtomicBool,
    producer_parked: AtomicBool,
    unpark_producer: AtomicBool,
    waiter_returned: AtomicBool,
    query_releases: AtomicU64,
    query_reacquires: AtomicU64,
    release_wait: AtomicU64,
    reacquire_wait: AtomicU64,
    invalid_context: AtomicBool,
    stop_watchdog: AtomicBool,
}
fn load(flag: &AtomicBool) -> bool {
    flag.load(Ordering::SeqCst)
}
fn set(flag: &AtomicBool) {
    flag.store(true, Ordering::SeqCst);
}
fn wait(label: &str, mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !ready() {
        assert!(Instant::now() < deadline, "insufficientexercise: {label}");
        std::thread::yield_now();
    }
}
fn typeck<'tcx>(tcx: TyCtxt<'tcx>, key: LocalDefId) -> &'tcx TypeckResults<'tcx> {
    if FACTS.armed.swap(false, Ordering::SeqCst) {
        set(&FACTS.producer_entered);
        // Provider code is outside query cache locks. Do not park after publishing
        // the result here: completion belongs to the query engine, not this hook.
        wait("controller did not release typeck provider", || {
            load(&FACTS.finish_query)
        });
    }
    TYPECK.get().unwrap()(tcx, key)
}
struct Selected {
    observer: Option<QueryAdmissionObserver>,
}
impl Callbacks for Selected {
    fn config(&mut self, config: &mut Config) {
        config.query_admission = self.observer.take();
        config.make_frontend_admission = Some(Box::new(|| {
            // The process jobserver may be larger than the two-worker frontend
            // (backend jobs also influence its capacity). Reserve REAL surplus
            // before starting pool workers, leaving exactly one external token
            // for the second frontend worker alongside the implicit allowance.
            let client = jobserver::client();
            let available = client.available().expect("native supply inventory");
            assert!(
                available >= 1,
                "insufficientexercise: no second worker supply"
            );
            let mut surplus = Vec::with_capacity(available - 1);
            for _ in 1..available {
                surplus.push(client.acquire().expect("reserve real surplus supply"));
            }
            assert_eq!(client.available().unwrap(), 1);
            *SURPLUS.lock().unwrap() = Some(surplus);
            let proxy = TicketedProxy::new_with_event_capacity(0);
            assert!(PROXY.set(Arc::clone(&proxy)).is_ok());
            let admission = FrontendAdmission::from_ticketed(proxy).with_unit_observer(
                || {
                    if DUPLICATE.with(Cell::get) {
                        UNIT_REACQUIRES.fetch_add(1, Ordering::SeqCst);
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
                    {
                        FACTS.invalid_context.store(true, Ordering::SeqCst);
                    }
                    if QUALIFIED_RELEASE.with(Cell::get) != Some(context.qualified_wait_id) {
                        FACTS.invalid_context.store(true, Ordering::SeqCst);
                    }
                    FACTS
                        .reacquire_wait
                        .store(context.qualified_wait_id.wait_id.get(), Ordering::SeqCst);
                    FACTS.query_reacquires.fetch_add(1, Ordering::SeqCst);
                },
                move |context| {
                    if !DUPLICATE.with(Cell::get) {
                        return;
                    }
                    if context.attachment_id != expected
                        || context.phase != AdmissionPhase::QueryWaitRelease
                    {
                        FACTS.invalid_context.store(true, Ordering::SeqCst);
                    }
                    QUALIFIED_RELEASE
                        .with(|released| released.set(Some(context.qualified_wait_id)));
                    FACTS
                        .release_wait
                        .store(context.qualified_wait_id.wait_id.get(), Ordering::SeqCst);
                    FACTS.query_releases.fetch_add(1, Ordering::SeqCst);
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
            .find(|id| tcx.item_name(id.to_def_id()).as_str() == "selected_target")
            .expect("fixture missing selected_target");
        let proxy = Arc::clone(PROXY.get().expect("explicit factory not called"));
        let client = jobserver::client();
        let qualified = tcx.sess.query_admission.is_some();
        let surplus = SURPLUS
            .lock()
            .unwrap()
            .take()
            .expect("factory surplus ownership");
        FACTS.armed.store(true, Ordering::SeqCst);
        let controller = std::thread::spawn(move || {
            let reserved_surplus = surplus.len();
            let baseline_releases = proxy.debug_snapshot().inventory.releases;
            set(&CONTROLLER_READY);
            wait("actual duplicate query release absent", || {
                FACTS.query_releases.load(Ordering::SeqCst) + UNIT_RELEASES.load(Ordering::SeqCst)
                    > 0
            });
            let released = proxy.debug_snapshot();
            assert_eq!(
                released.used, 1,
                "duplicate query must release native allowance"
            );
            assert!(released.inventory.releases > baseline_releases);
            assert_eq!(released.pending, 0);
            // This is the real native token released by the actual duplicate
            // query waiter, not a second semaphore or a synthetic grant.
            let held = client.acquire().expect("controller native acquire");
            assert_eq!(client.available().unwrap(), 0);
            set(&FACTS.finish_query);
            wait("producer did not complete actual query", || {
                load(&FACTS.producer_parked)
            });
            wait("query-ready contextual reacquire absent", || {
                FACTS.query_reacquires.load(Ordering::SeqCst)
                    + UNIT_REACQUIRES.load(Ordering::SeqCst)
                    > 0
            });
            wait("real TicketedProxy Pending reacquire absent", || {
                proxy.debug_snapshot().pending > 0
            });
            let pending = proxy.debug_snapshot();
            assert_eq!((pending.used, pending.pending), (1, 1));
            assert!(
                !load(&FACTS.waiter_returned),
                "query result alone authorized return"
            );
            assert!(!load(&FACTS.invalid_context));
            if qualified {
                assert!(FACTS.query_releases.load(Ordering::SeqCst) > 0);
                assert!(FACTS.query_reacquires.load(Ordering::SeqCst) > 0);
                assert_eq!(
                    FACTS.release_wait.load(Ordering::SeqCst),
                    FACTS.reacquire_wait.load(Ordering::SeqCst)
                );
            } else {
                assert_eq!(FACTS.query_releases.load(Ordering::SeqCst), 0);
                assert_eq!(FACTS.query_reacquires.load(Ordering::SeqCst), 0);
                assert!(UNIT_RELEASES.load(Ordering::SeqCst) > 0);
                assert!(UNIT_REACQUIRES.load(Ordering::SeqCst) > 0);
            }
            assert!(pending.events.is_empty(), "eventcap0 must stay disabled");
            assert!(pending.inventory.requests > 0);
            assert!(pending.inventory.releases > 0);
            assert_eq!(pending.inventory.helper_errors, 0);
            // Producer stays parked while only native supply can grant waiter.
            drop(held);
            wait("helper grant did not resume waiter", || {
                load(&FACTS.waiter_returned)
            });
            let resumed = proxy.debug_snapshot();
            assert!(resumed.inventory.grants > pending.inventory.grants);
            assert!(resumed.inventory.acks > pending.inventory.acks);
            assert!(resumed.inventory.supplies > pending.inventory.supplies);
            set(&FACTS.unpark_producer);
            eprintln!(
                "selected-admission-evidence: {{\"actual_query_release\":true,\"producer_completed\":true,\"producer_parked_outside_locks\":true,\"query_ready_reacquire\":true,\"proxy_pending\":true,\"returned_before_supply\":false,\"returned_after_supply\":true,\"event_capacity\":0,\"reserved_real_surplus\":{},\"qualified_context_observed\":{},\"pending_requests\":{},\"pending_grants\":{},\"pending_acks\":{},\"resumed_grants\":{},\"resumed_acks\":{}}}",
                reserved_surplus,
                qualified,
                pending.inventory.requests,
                pending.inventory.grants,
                pending.inventory.acks,
                resumed.inventory.grants,
                resumed.inventory.acks
            );
            surplus
        });
        wait("controller not ready", || load(&CONTROLLER_READY));
        par_join(
            || {
                tcx.typeck(owner);
                // Real result has been cached and query waiters notified. Park
                // only this outer closure, outside Query/Sleep/State locks.
                set(&FACTS.producer_parked);
                wait("controller did not unpark producer", || {
                    load(&FACTS.unpark_producer)
                });
            },
            || {
                wait("producer did not enter real typeck override", || {
                    load(&FACTS.producer_entered)
                });
                DUPLICATE.with(|active| active.set(true));
                tcx.typeck(owner);
                DUPLICATE.with(|active| active.set(false));
                set(&FACTS.waiter_returned);
            },
        );
        let surplus = controller.join().expect("selected controller failed");
        drop(surplus); // Restore every real surplus token only after the test joins.
        Compilation::Continue
    }
}
fn main() {
    // Own the existing native transport before any threads start. Invalid foreign
    // descriptors fail open in the existing validator, never select policy.
    let foreign_before = std::fs::read_link("/proc/self/fd/198").ok();
    let observer = unsafe {
        rustc_session::query_binding::admit_inherited(
            rustc_session::query_binding::protocol::RESERVED_TICKET_FD,
        )
    };
    if observer.is_none() {
        assert_eq!(
            foreign_before,
            std::fs::read_link("/proc/self/fd/198").ok(),
            "foreign descriptor changed by fail-open probe"
        );
    }
    // A bounded failure is not acceptance evidence. It is explicitly reported as
    // insufficientexercise, including deadlocks that prevent controller panic propagation.
    let watchdog = std::thread::spawn(|| {
        let deadline = Instant::now() + Duration::from_secs(90);
        while !load(&FACTS.stop_watchdog) {
            if Instant::now() >= deadline {
                eprintln!("insufficientexercise: selected admission fixture deadline");
                std::process::exit(86);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let args: Vec<String> = std::env::args().collect();
    rustc_driver::compiler_entrypoint(&args, &mut Selected { observer });
    set(&FACTS.stop_watchdog);
    watchdog.join().unwrap();
}
