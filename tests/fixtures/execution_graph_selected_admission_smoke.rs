#![feature(rustc_private)]
//! Compile-time smoke variants only, no private runtime selection argv or env.
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_session;

#[cfg(any(selected_serial_probe, selected_callback_panic))]
use rustc_data_structures::jobserver::{FrontendAdmission, TicketedProxy};
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface::{Compiler, Config};
use rustc_middle::ty::TyCtxt;
use rustc_session::query_admission::QueryAdmissionObserver;
use std::sync::atomic::{AtomicU64, Ordering};

static CALLS: AtomicU64 = AtomicU64::new(0);
struct Smoke {
    observer: Option<QueryAdmissionObserver>,
}
impl Callbacks for Smoke {
    fn config(&mut self, config: &mut Config) {
        config.query_admission = self.observer.take();
        // The default variant does not even assign the new field. It verifies
        // actual ordinary driver default behavior, also on the frozen baseline.
        #[cfg(any(selected_serial_probe, selected_callback_panic))]
        {
            config.make_frontend_admission = Some(Box::new(|| {
                CALLS.fetch_add(1, Ordering::SeqCst);
                FrontendAdmission::from_ticketed(TicketedProxy::new_with_event_capacity(0))
            }));
        }
    }
    fn after_expansion<'tcx>(&mut self, _: &Compiler, _: TyCtxt<'tcx>) -> Compilation {
        #[cfg(selected_callback_panic)]
        {
            assert_eq!(CALLS.load(Ordering::SeqCst), 1);
            panic!("selected-admission-intentional-callback-panic");
        }
        #[cfg(not(selected_callback_panic))]
        {
            // Both the default path and serial Some factory must never call
            // the explicit factory. Serial is tested without -Zthreads.
            assert_eq!(CALLS.load(Ordering::SeqCst), 0);
            eprintln!("selected-factory-smoke: calls=0");
            Compilation::Continue
        }
    }
}
fn main() {
    // As in the selected fixture, transport observation never chooses policy.
    let observer = unsafe {
        rustc_session::query_binding::admit_inherited(
            rustc_session::query_binding::protocol::RESERVED_TICKET_FD,
        )
    };
    let args: Vec<String> = std::env::args().collect();
    rustc_driver::compiler_entrypoint(&args, &mut Smoke { observer });
}
