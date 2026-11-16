//! Explicit scoped compiler storage for a serial resident request batch.
//!
//! Selected only by the explicit experimental owned resident transport.
//! The ordinary compiler and legacy resident path do not select this storage.
//! Retaining native owners does not authorize a query hit across revisions.

use std::cell::{Cell, RefCell};

use rustc_arena::TypedArena;
use rustc_data_structures::jobserver;
use rustc_middle::ty::{CurrentGcx, ResidentTypeDomain, with_resident_type_domain_and_owner};
use rustc_session::EarlyDiagCtxt;
use rustc_session::config;
use rustc_span::SessionGlobals;
use rustc_span::source_map::{RealFileLoader, SourceMapInputs};
use rustc_target::spec::Target;

use crate::interface::{Compiler, Config, execute_compiler, with_owned_compiler};
use crate::util;

// Field order matters: sessions and their diagnostics retire before globals.
struct BatchOwners {
    compilers: TypedArena<Compiler>,
    globals: RefCell<SessionGlobals>,
    current_gcx: CurrentGcx,
}

/// Storage scoped to one registered compiler worker and one native type universe.
/// Each invocation still constructs a fresh Session and query state, completes
/// diagnostics, and leaves incremental lock finalization to the normal pipeline.
/// No previous context may be re-entered through this API.
pub struct ResidentCompilerBatch<'domain> {
    domain: ResidentTypeDomain<'domain>,
    owners: &'domain BatchOwners,
    initial_options: config::Options,
    target: Target,
    extra_symbols: Vec<&'static str>,
    invocations: Cell<usize>,
}

impl<'domain> ResidentCompilerBatch<'domain> {
    /// Allocated invocation and context counts, not live query-cache hit counts.
    pub fn counts(&self) -> (usize, usize) {
        (self.invocations.get(), self.domain.allocated_context_count())
    }

    /// Execute one fresh revision using batch-owned native storage. The caller
    /// must validate source and identity universes before reusing typed results.
    /// A failed invocation still consumes its storage until the batch retires.
    // JUSTIFICATION: validate configuration before constructing a session.
    #[allow(rustc::bad_opt_access)]
    pub fn run<R>(
        &self,
        config: Config,
        f: impl FnOnce(&'domain Compiler, &ResidentTypeDomain<'domain>) -> R,
    ) -> R {
        let early_dcx = EarlyDiagCtxt::new(config.opts.error_format);
        reject_unsupported(&early_dcx, &config);
        let target = config::build_target_config(
            &early_dcx,
            &config.opts.target_triple,
            config.opts.sysroot.path(),
            config.opts.unstable_opts.unstable_options,
        );
        let initial = &self.initial_options;
        if config.opts.edition != initial.edition
            || target != self.target
            || config.opts.remap_path_prefix != initial.remap_path_prefix
            || config.opts.remap_path_scope != initial.remap_path_scope
            || config.opts.unstable_opts.remap_cwd_prefix != initial.unstable_opts.remap_cwd_prefix
            || config.opts.unstable_opts.src_hash_algorithm(&target)
                != initial.unstable_opts.src_hash_algorithm(&self.target)
            || config.opts.unstable_opts.checksum_hash_algorithm()
                != initial.unstable_opts.checksum_hash_algorithm()
            || config.extra_symbols != self.extra_symbols
        {
            early_dcx.early_fatal("resident batch global configuration cannot change");
        }
        let count = self.invocations.get();
        if count >= 64 {
            early_dcx.early_fatal("resident batch invocation limit reached; retire this batch");
        }
        self.invocations.set(count + 1);
        self.owners.globals.borrow_mut().enter_resident_revision(|| {
            let source_map = rustc_span::source_map::get_source_map()
                .expect("resident batch owns a native source map");
            let _source_revision = source_map
                .begin_resident_source_revision(8192, 8 * 1024 * 1024, 64)
                .unwrap_or_else(|error| {
                    early_dcx.early_fatal(format!("resident source revision refused: {error}"))
                });
            with_owned_compiler(
                config,
                target,
                self.owners.current_gcx.clone(),
                None,
                None,
                |compiler| {
                    let compiler: &'domain Compiler = self.owners.compilers.alloc(compiler);
                    execute_compiler(compiler, |_| f(compiler, &self.domain))
                },
            )
        })
    }
}

// Explicitly exclude observer/callback configurations whose ordinary lifecycle
// requires a per-invocation pool seal, a custom source owner, or admission scope.
#[allow(rustc::bad_opt_access)]
fn reject_unsupported(early_dcx: &EarlyDiagCtxt, config: &Config) {
    if config.opts.incremental.is_some() {
        early_dcx.early_fatal(
            "resident owned batch incremental persistence requires an active-revision source-file view",
        );
    }
    if config.query_admission.is_some()
        || config.make_frontend_admission.is_some()
        || config.file_loader.is_some()
    {
        early_dcx.early_fatal(
            "resident batch does not support query capture, custom admission, or custom file loaders",
        );
    }
}

/// Enter one experimental serial batch. The initial configuration is returned
/// to the continuation for its first invocation. All native context cells and
/// typed arenas retire before the Session/global owners and worker return.
///
/// This function does not establish changed-source compatibility, a memory-byte
/// budget, managed-hosted integration, or live query-cache reuse. It must not be
/// selected as a production default on the strength of owner construction alone.
// JUSTIFICATION: before session exists, only configuration is available.
#[allow(rustc::bad_opt_access)]
pub fn with_resident_compiler_batch<R: Send>(
    config: Config,
    f: impl for<'domain> FnOnce(ResidentCompilerBatch<'domain>, Config) -> R + Send,
) -> R {
    let early_dcx = EarlyDiagCtxt::new(config.opts.error_format);
    reject_unsupported(&early_dcx, &config);
    if config.opts.jobs.frontend.is_some() {
        early_dcx.early_fatal("resident batch requires serial frontend mode");
    }
    rustc_data_structures::sync::set_dyn_thread_safe_mode(false);
    let jobs = config.opts.jobs;
    if let Some(limit) = jobs.frontend.max(jobs.backend).max(jobs.linker.limit()) {
        jobserver::initialize(limit.get(), |err| {
            early_dcx
                .early_struct_warn(err)
                .with_note("the build environment is likely misconfigured")
                .emit()
        });
    }
    crate::callbacks::setup_callbacks();
    // Proc-macro dylibs are not revision-isolated in this experimental owner.
    // Use the same irreversible native guard as the resident transport.
    rustc_session::resident::enable_process();
    let target = config::build_target_config(
        &early_dcx,
        &config.opts.target_triple,
        config.opts.sysroot.path(),
        config.opts.unstable_opts.unstable_options,
    );
    let initial_options = config.opts.clone();
    let extra_symbols = config.extra_symbols.clone();
    let inputs = SourceMapInputs {
        file_loader: Box::new(RealFileLoader),
        path_mapping: config.opts.file_path_mapping(),
        hash_kind: config.opts.unstable_opts.src_hash_algorithm(&target),
        checksum_hash_kind: config.opts.unstable_opts.checksum_hash_algorithm(),
    };
    util::run_in_serial_worker_with_owned_globals(
        &early_dcx,
        config.opts.edition,
        jobs,
        &extra_symbols,
        inputs,
        |globals| {
            let owners = BatchOwners {
                compilers: TypedArena::default(),
                globals: RefCell::new(globals),
                current_gcx: CurrentGcx::new(),
            };
            with_resident_type_domain_and_owner(owners, |domain, owners| {
                f(
                    ResidentCompilerBatch {
                        domain,
                        owners,
                        initial_options,
                        target,
                        extra_symbols: extra_symbols.clone(),
                        invocations: Cell::new(0),
                    },
                    config,
                )
            })
        },
    )
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use rustc_session::config::{Input, Options};
    use rustc_span::FileName;
    use rustc_structures::CrateType;

    use super::*;

    // A self-contained no_core fixture exercises actual native analysis without
    // accepting metadata from a different prebuilt standard-library version.
    #[allow(rustc::bad_opt_access)]
    fn config(revision: usize) -> Config {
        static INTERNAL: AtomicBool = AtomicBool::new(false);
        let mut opts = Options::default();
        opts.unstable_features = rustc_feature::UnstableFeatures::Allow;
        opts.crate_types = vec![CrateType::Rlib];
        Config {
            query_admission: None,
            opts,
            crate_cfg: Vec::new(),
            crate_check_cfg: Vec::new(),
            input: Input::Str {
                name: FileName::Custom(format!("resident-revision-{revision}.rs")),
                input: format!(
                    "#![feature(no_core, lang_items)]\n#![no_core]\n#![allow(internal_features)]\n\
                     #[lang = \"pointee_sized\"] pub trait PointeeSized {{}}\n\
                     #[lang = \"meta_sized\"] pub trait MetaSized: PointeeSized {{}}\n\
                     #[lang = \"sized\"] pub trait Sized: MetaSized {{}}\n\
                     pub fn value() -> bool {{ {} }}",
                    revision == 0
                ),
            },
            output_dir: None,
            output_file: None,
            ice_file: None,
            file_loader: None,
            lint_caps: Default::default(),
            psess_created: None,
            track_state: None,
            register_lints: None,
            override_queries: None,
            extra_symbols: Vec::new(),
            make_codegen_backend: None,
            make_frontend_admission: None,
            using_internal_features: &INTERNAL,
        }
    }

    #[test]
    fn resident_batch_keeps_project_typeck_alive_across_fresh_contexts() {
        let first = config(0);
        let second = config(1);
        with_resident_compiler_batch(first, |batch, first| {
            let mut retained = Vec::new();
            for config in [first, second] {
                let value = batch.run(config, |compiler, domain| {
                    let krate = crate::passes::parse(&compiler.sess);
                    let (value, locks) = crate::passes::create_and_enter_global_ctxt_in_domain(
                        compiler,
                        domain,
                        krate,
                        |tcx| {
                            tcx.ensure_ok().analysis(());
                            let owner = tcx.hir_body_owners().next().expect("fixture function");
                            let body = tcx.hir_body_owned_by(owner);
                            let results = tcx.typeck(owner);
                            let ty = results.expr_ty(body.value);
                            assert_eq!(ty, tcx.types.bool);
                            (results, ty)
                        },
                    )
                    .expect("available context storage");
                    assert!(locks.is_none(), "test must not retain filesystem locks");
                    value
                });
                retained.push(value);
                for (results, ty) in &retained {
                    assert!(!results.node_types().items_in_stable_order().is_empty());
                    assert!(ty.is_bool());
                }
            }
            assert!(!std::ptr::eq(retained[0].0, retained[1].0));
            assert_eq!(retained[0].1, retained[1].1);
        });
    }
}
