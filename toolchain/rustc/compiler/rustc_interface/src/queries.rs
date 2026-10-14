use std::any::Any;
use std::sync::Arc;

use rustc_codegen_ssa::traits::CodegenBackend;
use rustc_codegen_ssa::{CompiledModules, CrateInfo};
use rustc_data_structures::svh::Svh;
use rustc_errors::timings::TimingSection;
use rustc_hir::def_id::LOCAL_CRATE;
use rustc_metadata::EncodedMetadata;
use rustc_middle::dep_graph::{DepGraph, WorkProduct, WorkProductMap};
use rustc_middle::ty::TyCtxt;
use rustc_session::config::{self, OutputFilenames, OutputType};
use rustc_session::{IncrCompSession, Session};

use crate::diagnostics::FailedWritingFile;
use crate::passes;

pub struct Linker {
    dep_graph: DepGraph,
    output_filenames: Arc<OutputFilenames>,
    // Only present when incr. comp. is enabled.
    crate_hash: Option<Svh>,
    crate_info: CrateInfo,
    metadata: EncodedMetadata,
    ongoing_codegen: Box<dyn Any>,
}

/// justrust: codegen joined and the work-product index saved, ready to link.
/// `JoinedLinker::link` needs no `TyCtxt`, so it can run in parallel with
/// `TyCtxt::finish` (dep-graph and query-cache persist). The incremental
/// session directory is finalized by the caller after both are done.
pub struct JoinedLinker {
    pub crate_hash: Option<Svh>,
    output_filenames: Arc<OutputFilenames>,
    crate_info: CrateInfo,
    metadata: EncodedMetadata,
    compiled_modules: CompiledModules,
}

/// justrust: overlap the incremental persist with the link
/// (`JUSTRUST_RUSTC_OVERLAP_PERSIST=0` turns it off). Only taken when the
/// session can run work on two threads (`-Zthreads` > 1), incremental is on,
/// and the plain link path applies.
pub fn overlap_persist_enabled(sess: &Session) -> bool {
    std::env::var_os("JUSTRUST_RUSTC_OVERLAP_PERSIST").is_none_or(|v| v != "0")
        && rustc_data_structures::sync::is_dyn_thread_safe()
        && sess.opts.incremental.is_some()
        && !sess.opts.unstable_opts.no_link
        && !sess.opts.unstable_opts.offload.iter().any(|o| matches!(o, config::Offload::HostMetadata(_)))
        && sess.opts.output_types.keys().any(|&i| i == OutputType::Exe || i == OutputType::Metadata)
}

impl Linker {
    pub fn codegen_and_build_linker(
        tcx: TyCtxt<'_>,
        codegen_backend: &dyn CodegenBackend,
    ) -> Linker {
        let (ongoing_codegen, crate_info, metadata) = passes::start_codegen(codegen_backend, tcx);

        Linker {
            dep_graph: tcx.dep_graph.clone(),
            output_filenames: Arc::clone(tcx.output_filenames(())),
            crate_hash: if tcx.sess.opts.incremental.is_some() {
                Some(tcx.crate_hash(LOCAL_CRATE))
            } else {
                None
            },
            crate_info,
            metadata,
            ongoing_codegen,
        }
    }

    pub fn link(
        self,
        sess: &Session,
        incr_comp_session: Option<IncrCompSession>,
        codegen_backend: &dyn CodegenBackend,
    ) {
        let output_filenames = Arc::clone(&self.output_filenames);
        let crate_hash = self.crate_hash;
        let (compiled_modules, crate_info, metadata) =
            self.join_inner(sess, incr_comp_session.as_ref(), codegen_backend);

        // Now that we won't touch anything in the incremental compilation directory
        // any more, we can finalize it (which involves renaming it)
        rustc_incremental::finalize_session_directory(sess, incr_comp_session, crate_hash);

        JoinedLinker { crate_hash, output_filenames, crate_info, metadata, compiled_modules }
            .link(sess, codegen_backend)
    }

    /// justrust: everything `link` does before finalizing the session
    /// directory (see `JoinedLinker`).
    pub fn join(
        self,
        sess: &Session,
        incr_comp_session: Option<&IncrCompSession>,
        codegen_backend: &dyn CodegenBackend,
    ) -> JoinedLinker {
        let output_filenames = Arc::clone(&self.output_filenames);
        let crate_hash = self.crate_hash;
        let (compiled_modules, crate_info, metadata) =
            self.join_inner(sess, incr_comp_session, codegen_backend);
        JoinedLinker { crate_hash, output_filenames, crate_info, metadata, compiled_modules }
    }

    fn join_inner(
        self,
        sess: &Session,
        incr_comp_session: Option<&IncrCompSession>,
        codegen_backend: &dyn CodegenBackend,
    ) -> (CompiledModules, CrateInfo, EncodedMetadata) {
        let (compiled_modules, mut work_products) = sess.time("finish_ongoing_codegen", || {
            match self.ongoing_codegen.downcast::<CompiledModules>() {
                // This was a check only build
                Ok(compiled_modules) => (*compiled_modules, WorkProductMap::default()),

                Err(ongoing_codegen) => codegen_backend.join_codegen(
                    ongoing_codegen,
                    sess,
                    incr_comp_session,
                    &self.output_filenames,
                    &self.crate_info,
                ),
            }
        });

        if sess.codegen_units().as_usize() == 1 && sess.opts.unstable_opts.time_llvm_passes {
            codegen_backend.print_pass_timings()
        }

        if sess.print_llvm_stats() {
            codegen_backend.print_statistics()
        }

        if let Some(out_path) = sess.print_llvm_stats_json() {
            let llvm_stats_json = codegen_backend.print_statistics_json();

            if !llvm_stats_json.is_empty() {
                if let Err(e) = std::fs::write(&out_path, llvm_stats_json) {
                    sess.dcx().err(format!("failed to write stats to {}: {}", out_path, e));
                }
            } else {
                sess.dcx().warn(format!(
                    "requested to print LLVM statistics to JSON file {}, but the codegen backend \
                    did not provide any statistics",
                    out_path,
                ));
            }
        }

        sess.timings.end_section(sess.dcx(), TimingSection::Codegen);

        if sess.opts.incremental.is_some()
            && let Some(path) = self.metadata.path()
        {
            let (id, product) = rustc_incremental::copy_cgu_workproduct_to_incr_comp_cache_dir(
                sess,
                incr_comp_session.unwrap(),
                WorkProduct::METADATA_WORKPRODUCT_CGU_NAME,
                &[(OutputType::Metadata.extension(), path)],
            );
            work_products.insert(id, product);
        }

        if let Some(guar) = sess.dcx().has_errors_or_delayed_bugs() {
            guar.raise_fatal();
        }

        sess.time("serialize_work_products", || {
            rustc_incremental::save_work_product_index(
                sess,
                incr_comp_session,
                &self.dep_graph,
                work_products,
            )
        });

        let prof = sess.prof.clone();
        prof.generic_activity("drop_dep_graph").run(move || drop(self.dep_graph));

        (compiled_modules, self.crate_info, self.metadata)
    }
}

impl JoinedLinker {
    pub fn link(self, sess: &Session, codegen_backend: &dyn CodegenBackend) {
        let _timer = sess.timer("link");
        let compiled_modules = self.compiled_modules;

        // The `HostMetadata` offload pass only writes the kernel manifest.
        // Codegen was already skipped so there are no files to link.
        if sess
            .opts
            .unstable_opts
            .offload
            .iter()
            .any(|o| matches!(o, config::Offload::HostMetadata(_)))
        {
            return;
        }

        if !sess
            .opts
            .output_types
            .keys()
            .any(|&i| i == OutputType::Exe || i == OutputType::Metadata)
        {
            return;
        }

        if sess.opts.unstable_opts.no_link {
            let rlink_file = self.output_filenames.with_extension(config::RLINK_EXT);
            CompiledModules::serialize_rlink(
                sess,
                &rlink_file,
                &compiled_modules,
                &self.crate_info,
                &self.metadata,
                &self.output_filenames,
            )
            .unwrap_or_else(|error| {
                sess.dcx().emit_fatal(FailedWritingFile { path: &rlink_file, error })
            });
            return;
        }

        let _timer = sess.prof.verbose_generic_activity("link_crate");
        let _timing = sess.timings.section_guard(sess.dcx(), TimingSection::Linking);
        codegen_backend.link(
            sess,
            compiled_modules,
            self.crate_info,
            self.metadata,
            &self.output_filenames,
        )
    }
}

/// justrust: `TyCtxt::finish` (dep-graph and query-cache persist) on one
/// thread, joining codegen and linking on another. Returns the crate hash
/// for `finalize_after_overlap`. Errors and fatal errors propagate as in the
/// serial path: joining raises a fatal error if compilation failed, and the
/// persist skips writing when errors were emitted.
pub fn overlap_persist_and_link(
    tcx: TyCtxt<'_>,
    linker: Linker,
    codegen_backend: &dyn CodegenBackend,
) -> Option<Svh> {
    let sess = tcx.sess;
    let crate_hash = linker.crate_hash;
    let work = rustc_data_structures::marker::IntoDynSyncSend(AssertSend((linker, codegen_backend)));
    rustc_data_structures::sync::par_join(
        move || tcx.finish(),
        move || {
            // Move the whole wrapper (edition 2021 closures would otherwise
            // capture the non-`DynSend` fields one by one).
            let work = work;
            let (linker, backend) = work.0.0;
            let joined = linker.join(sess, tcx.incr_comp_session, backend);
            joined.link(sess, backend);
            // justrust: the outputs are complete; reply now and let the
            // persist finish in the background (`JUSTRUST_RUSTC_EARLY_REPLY_FD`).
            if let Some(incr) = tcx.incr_comp_session
                && std::env::var_os("JUSTRUST_RUSTC_EARLY_REPLY_FD").is_some()
                && sess.dcx().has_errors_or_delayed_bugs().is_none()
            {
                let marker = incr.new_session_directory.join(rustc_incremental::JR_PERSISTING);
                if std::fs::write(&marker, b"").is_ok() && !sess.jr_early_reply() {
                    let _ = std::fs::remove_file(&marker);
                }
            }
        },
    );
    crate_hash
}

/// justrust: the `Linker` (its `Box<dyn Any>` ongoing codegen is a handle
/// to codegen worker threads) and the `CodegenBackend` (used through `&self`)
/// move to the joining and linking thread and are used by it alone while the
/// other thread persists.
struct AssertSend<T>(T);
unsafe impl<T> Send for AssertSend<T> {}
unsafe impl<T> Sync for AssertSend<T> {}

/// justrust: rename the session directory after an overlapped persist and
/// link. Skipped when any error was emitted (the serial path finalizes
/// before linking, so a failed link leaves this session unpublished and the
/// next build reuses the previous one: slower, never wrong).
pub fn finalize_after_overlap(
    sess: &Session,
    incr_comp_session: Option<IncrCompSession>,
    crate_hash: Option<Svh>,
) {
    // The `JR_PERSISTING` marker (early reply) stays until the session
    // directory is renamed and unlocked: the next rustc waits for that.
    let marked = incr_comp_session.as_ref().and_then(|incr| {
        let dir: &std::path::Path = &incr.new_session_directory;
        dir.join(rustc_incremental::JR_PERSISTING).exists().then(|| {
            let name = dir.file_name()?.to_str()?.strip_suffix("working")?.to_owned();
            Some((dir.parent()?.to_owned(), name))
        })?
    });
    if sess.dcx().has_errors_or_delayed_bugs().is_none() {
        rustc_incremental::finalize_session_directory(sess, incr_comp_session, crate_hash);
    }
    if let Some((crate_dir, prefix)) = marked
        && let Ok(entries) = std::fs::read_dir(&crate_dir)
    {
        for e in entries.flatten() {
            if e.file_name().to_str().is_some_and(|n| n.starts_with(&prefix)) {
                let _ = std::fs::remove_file(e.path().join(rustc_incremental::JR_PERSISTING));
            }
        }
    }
}
