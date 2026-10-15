//! Actual-source observer driver. Controls arrive on an owned fixture descriptor.
#![feature(rustc_private)]
extern crate rustc_driver;
extern crate rustc_interface;
#[cfg(frontend_scope_api)]
extern crate rustc_middle;
extern crate rustc_session;

use rustc_driver::Callbacks;
use rustc_interface::interface::Config;
use rustc_session::query_admission::QueryAdmissionObserver;
use std::io::Read;
use std::os::fd::FromRawFd;

struct Mapping {
    observer: Option<QueryAdmissionObserver>,
}
impl Callbacks for Mapping {
    fn config(&mut self, config: &mut Config) {
        config.query_admission = self.observer.take();
    }

    #[cfg(frontend_scope_api)]
    fn after_expansion<'tcx>(
        &mut self,
        _: &rustc_interface::interface::Compiler,
        tcx: rustc_middle::ty::TyCtxt<'tcx>,
    ) -> rustc_driver::Compilation {
        if let Some(observer) = &tcx.sess.query_admission {
            if let Some(pool) = observer.resource_pool() {
                assert!(
                    pool.current_point().is_ok(),
                    "native frontend root must be active"
                );
                assert!(
                    observer.enter_frontend_scope().is_none(),
                    "nested root rejected"
                );
                assert!(
                    observer.enter_frontend_scope().is_none(),
                    "repeat nested root rejected"
                );
            }
        }
        rustc_driver::Compilation::Continue
    }
}
fn main() {
    let controls = {
        // SAFETY: this fixture's parent exclusively reserves fd198, passes only
        // that owned read endpoint, and never overwrites a preexisting fd198.
        // Consume and close it before compiler entry or any compiler threads.
        let mut control = unsafe { std::fs::File::from_raw_fd(198) };
        let mut text = String::new();
        control
            .read_to_string(&mut text)
            .expect("fixture control descriptor");
        text
    };
    let mut lines = controls.lines();
    let capture = lines.next().expect("capture path or '-' required");
    let capacity = lines.next().expect("capacity or 'default' required");
    // Old default controls stay exactly two lines. Timing is a separate opt-in.
    let timing = lines.next().unwrap_or("off");
    assert!(matches!(timing, "off" | "timing"), "invalid timing control");
    let dispatch = lines.next().unwrap_or("off");
    assert!(
        matches!(dispatch, "off" | "dispatch"),
        "invalid dispatch control"
    );
    assert!(lines.next().is_none(), "unexpected fixture controls");
    let observer = if capture == "-" {
        None
    } else if dispatch == "dispatch" {
        #[cfg(frontend_dispatch_api)]
        {
            Some(
                QueryAdmissionObserver::new_with_event_capacity_and_options(
                    std::path::Path::new(capture),
                    capacity.parse().expect("numeric dispatch capacity"),
                    timing == "timing",
                    true,
                )
                .expect("private dispatch capture rejected"),
            )
        }
        #[cfg(not(frontend_dispatch_api))]
        panic!("dispatch requires matching rebuilt private metadata");
    } else if timing == "timing" {
        #[cfg(frontend_timing_api)]
        {
            Some(
                QueryAdmissionObserver::new_with_event_capacity_and_timing(
                    std::path::Path::new(capture),
                    capacity.parse().expect("numeric timing capacity"),
                )
                .expect("private timed capture rejected"),
            )
        }
        #[cfg(not(frontend_timing_api))]
        panic!("timing requires matching rebuilt private metadata");
    } else if capacity == "default" {
        Some(
            QueryAdmissionObserver::new(std::path::Path::new(capture))
                .expect("private capture directory rejected"),
        )
    } else {
        #[cfg(frontend_large_capacity)]
        {
            Some(
                QueryAdmissionObserver::new_with_event_capacity(
                    std::path::Path::new(capture),
                    capacity.parse().expect("numeric capacity"),
                )
                .expect("private large capture rejected"),
            )
        }
        #[cfg(not(frontend_large_capacity))]
        panic!("large capacity requires matching rebuilt private metadata");
    };
    // No provider overrides, forced queries, private argv or private environment.
    let compiler_arguments: Vec<String> = std::env::args().collect();
    rustc_driver::compiler_entrypoint(&compiler_arguments, &mut Mapping { observer });
}
