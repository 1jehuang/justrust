#![feature(rustc_private)]

extern crate rustc_driver;
extern crate rustc_interface;
extern crate rustc_middle;
extern crate rustc_span;

use rustc_driver::{Callbacks, Compilation};
use rustc_interface::interface;
use rustc_middle::ty::TyCtxt;
#[cfg(not(legacy_checksum_fixture))]
use rustc_span::{SourceFileHash, SourceFileHashAlgorithm, Symbol};

struct ReadBoundary;

#[cfg(not(legacy_checksum_fixture))]
impl Callbacks for ReadBoundary {
    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        let mode = std::env::var("READ_BOUNDARY_MODE").unwrap();
        let path = Symbol::intern("payload.bin");
        let sess = tcx.sess;
        if mode == "default-off" {
            assert!(sess.tracked_binary_read_checksums.borrow().is_empty());
            sess.record_binary_file_read(path, b"ignored");
            assert!(sess.tracked_binary_read_checksums.borrow().is_empty());
        } else {
            let expected =
                Some((3, SourceFileHash::new_in_memory(SourceFileHashAlgorithm::Sha256, b"old")));
            assert_eq!(sess.tracked_binary_read_checksums.borrow().get(&path), Some(&expected));
        }
        match mode.as_str() {
            "mutate" | "default-off" => std::fs::write("payload.bin", b"new content").unwrap(),
            "delete" => std::fs::remove_file("payload.bin").unwrap(),
            "identical" => {
                let bytes = std::fs::read("payload.bin").unwrap();
                sess.record_binary_file_read(path, &bytes);
                assert!(sess.tracked_binary_read_checksums.borrow()[&path].is_some());
            }
            "different" => {
                std::fs::write("payload.bin", b"new content").unwrap();
                let bytes = std::fs::read("payload.bin").unwrap();
                sess.record_binary_file_read(path, &bytes);
                assert_eq!(sess.tracked_binary_read_checksums.borrow()[&path], None);
                sess.record_binary_file_read(path, b"old");
                assert_eq!(sess.tracked_binary_read_checksums.borrow()[&path], None);
            }
            "untracked" => {
                sess.file_depinfo.borrow_mut().insert(Symbol::intern("untracked.bin"));
                sess.file_depinfo.borrow_mut().insert(Symbol::intern("missing.bin"));
                std::fs::write("untracked.bin", b"later").unwrap();
            }
            "unchanged" => {}
            _ => panic!("unknown fixture mode: {mode}"),
        }
        Compilation::Continue
    }
}

// The same read/emit boundary can demonstrate the bug with an older checkout compiler.
#[cfg(legacy_checksum_fixture)]
impl Callbacks for ReadBoundary {
    fn after_expansion<'tcx>(
        &mut self,
        _compiler: &interface::Compiler,
        _tcx: TyCtxt<'tcx>,
    ) -> Compilation {
        std::fs::write("payload.bin", b"new content").unwrap();
        Compilation::Continue
    }
}

fn main() {
    rustc_driver::compiler_entrypoint(&std::env::args().collect::<Vec<_>>(), &mut ReadBoundary);
}
