#![feature(rustc_private)]
extern crate rustc_data_structures;
extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;

use rustc_data_structures::sync::par_join;
use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface::Compiler;
use rustc_middle::ty::TyCtxt;

struct NestedQueries;
impl Callbacks for NestedQueries {
    fn after_expansion<'tcx>(&mut self, _: &Compiler, tcx: TyCtxt<'tcx>) -> Compilation {
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
        // Cached request outside any inner join remains on the actual outer
        // continuation. Queries before pool attachment elsewhere stay unbound.
        tcx.typeck(owners[0]);
        tcx.typeck(owners[7]);
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
