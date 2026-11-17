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
    options: ResidentBatchOptions,
}

/// Development-only selection. Ordinary owned batches retain their existing
/// nonincremental behavior. The control enables persistence without typed reuse.
#[derive(Clone, Copy, Default)]
pub struct ResidentBatchOptions {
    pub incremental_control: bool,
    pub live_queries: bool,
    /// Separately gated primitive MIR leaf closure experiment.
    pub selective_leaf: bool,
    /// Separately gated clean ASCII token retention, never a default.
    pub lexer_cache: bool,
    /// Separately gated pre-expansion AST retention, never a default.
    pub ast_cache: bool,
}

impl<'domain> ResidentCompilerBatch<'domain> {
    /// Allocated invocation and context counts, not live query-cache hit counts.
    pub fn counts(&self) -> (usize, usize) {
        (self.invocations.get(), self.domain.allocated_context_count())
    }

    pub fn finish_live_revision(&self, success: bool) {
        if let Some(state) = self.domain.live_query_state() {
            state.finish(success);
        }
    }

    pub fn ast_stats(&self) -> rustc_parse::resident_ast::AstCacheStats {
        rustc_parse::resident_ast::stats()
    }

    pub fn lexer_stats(&self) -> rustc_parse::resident::LexerCacheStats {
        rustc_parse::resident::stats()
    }

    pub fn live_stats(&self) -> rustc_middle::ty::resident_live::LiveQueryStats {
        self.domain.live_query_state().map(|state| state.stats()).unwrap_or_default()
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
        reject_unsupported(&early_dcx, &config, self.options);
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
fn reject_unsupported(early_dcx: &EarlyDiagCtxt, config: &Config, options: ResidentBatchOptions) {
    if config.opts.incremental.is_some() && !(options.incremental_control || options.live_queries) {
        early_dcx.early_fatal(
            "resident owned batch incremental persistence requires an active-revision source-file view",
        );
    }
    if options.selective_leaf && !options.live_queries {
        early_dcx.early_fatal("resident leaf closure experiment requires native typed state");
    }
    if options.live_queries && config.opts.incremental.is_none() {
        early_dcx.early_fatal("resident typed reuse requires an explicit incremental directory");
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
    with_resident_compiler_batch_options(config, ResidentBatchOptions::default(), f)
}

#[allow(rustc::bad_opt_access)]
pub fn with_resident_compiler_batch_options<R: Send>(
    config: Config,
    options: ResidentBatchOptions,
    f: impl for<'domain> FnOnce(ResidentCompilerBatch<'domain>, Config) -> R + Send,
) -> R {
    let early_dcx = EarlyDiagCtxt::new(config.opts.error_format);
    reject_unsupported(&early_dcx, &config, options);
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
            rustc_parse::resident_ast::with_resident_ast_cache(options.ast_cache, || {
                rustc_parse::resident::with_resident_lexer_cache(options.lexer_cache, || {
                    with_resident_type_domain_and_owner(owners, |domain, owners| {
                        domain.enable_live_queries(options.live_queries);
                        if let Some(state) = domain.live_query_state() {
                            state.enable_leaf(options.selective_leaf);
                        }
                        f(
                            ResidentCompilerBatch {
                                domain,
                                owners,
                                initial_options,
                                target,
                                extra_symbols: extra_symbols.clone(),
                                invocations: Cell::new(0),
                                options,
                            },
                            config,
                        )
                    })
                })
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
    fn resident_batch_derives_exact_file_revision_identity() {
        use rustc_middle::ty::resident_compiler_identity::{
            FrozenSemanticInvocation, build_revision_identity,
        };
        use rustc_middle::ty::resident_identity::{Compatibility, IdentityBudget, RevisionGate};
        use std::io::Write;

        struct Fixture(std::path::PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let path = std::env::temp_dir().join(format!(
            "resident_identity_{}_{}.rs",
            std::process::id(),
            rand::random::<u128>()
        ));
        let mut file =
            std::fs::OpenOptions::new().create_new(true).write(true).open(&path).unwrap();
        let fixture = Fixture(path);
        let source = |revision| match config(revision).input {
            Input::Str { input, .. } => input,
            _ => unreachable!(),
        };
        file.write_all(source(0).as_bytes()).unwrap();
        drop(file);
        let make_config = || {
            let mut config = config(0);
            config.input = Input::File(fixture.0.clone());
            config
        };
        let first = make_config();
        with_resident_compiler_batch(first, |batch, first| {
            let budget = IdentityBudget {
                max_crates: 1,
                max_definitions: 128,
                max_identity_bytes: 1024 * 1024,
            };
            let mut identities = Vec::new();
            for (revision, config) in [first, make_config(), make_config()].into_iter().enumerate()
            {
                if revision == 2 {
                    std::fs::write(&fixture.0, source(1)).unwrap();
                }
                let identity = batch.run(config, |compiler, domain| {
                    let krate = crate::passes::parse(&compiler.sess);
                    let (identity, locks) =
                        crate::passes::create_and_enter_global_ctxt_in_domain(
                            compiler,
                            domain,
                            krate,
                            |tcx| {
                                let prefix_budget = rustc_middle::ty::resident_definition_prefix::PrefixBudget {
                                    max_definitions: budget.max_definitions,
                                    max_bytes: budget.max_identity_bytes,
                                };
                                let before = rustc_middle::ty::resident_definition_prefix::capture_definition_prefix(tcx, prefix_budget)
                                    .expect("bounded native definition prefix before analysis");
                                tcx.ensure_ok().analysis(());
                                let after = rustc_middle::ty::resident_definition_prefix::capture_definition_prefix(tcx, prefix_budget)
                                    .expect("bounded native definition prefix after analysis");
                                assert!(before.is_preserved_by(&after));
                                build_revision_identity(tcx, FrozenSemanticInvocation {
                                namespace: b"native-fixed-no-core-test-v1",
                                options: b"fixed-config(0),same-target,rlib,allow-internal-no-core",
                                body_revision: b"no-additional-semantic-inputs",
                            }, budget).expect("complete exact local file identity")
                            },
                        )
                        .expect("available context storage");
                    assert!(locks.is_none());
                    identity
                });
                identities.push(identity);
            }
            assert_eq!(identities[0], identities[1]);
            assert!(identities[0].authorize_reuse(&identities[1], budget).is_ok());
            assert_eq!(
                identities[1].compatibility(&identities[2], budget),
                Compatibility::IdentityOnly(RevisionGate::SourceChanged)
            );
            assert!(identities[1].authorize_reuse(&identities[2], budget).is_err());
        });
    }

    #[test]
    fn resident_batch_captures_actual_loaded_external_metadata() {
        use rustc_metadata::creader::CStore;
        use rustc_metadata::resident_metadata::{
            ExternalSourceSnapshotError, ExternalSourceSnapshotLimits, MetadataSnapshotError,
            MetadataSnapshotLimits, with_resident_external_decoder_cache,
        };
        use rustc_middle::ty::resident_compiler_identity::{
            CompilerIdentityError, FrozenSemanticInvocation, build_revision_identity,
        };
        use rustc_middle::ty::resident_identity::IdentityBudget;

        // Unit executables live below stageN-rustc, not the compiler's bin/.
        // Default DLL-based sysroot discovery therefore cannot locate this std.
        // Select the sibling bootstrap sysroot, never an installed stock toolchain.
        let executable = std::env::current_exe().expect("native test executable");
        let sysroot = executable
            .ancestors()
            .find_map(|path| {
                let stage = path.file_name()?.to_str()?.strip_suffix("-rustc")?;
                let number = stage.strip_prefix("stage")?;
                if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                    return None;
                }
                Some(path.with_file_name(stage))
            })
            .expect("run the loaded-metadata acceptance with bootstrap x test");
        assert!(
            sysroot.join("lib/rustlib").is_dir(),
            "build matching standard libraries with x build --stage 1 library first"
        );
        let make_config = || {
            let mut config = config(0);
            config.opts.sysroot = rustc_session::config::Sysroot::new(Some(sysroot.clone()));
            config.input = Input::Str {
                name: FileName::Custom("resident-external-metadata.rs".into()),
                input: "pub fn value() -> bool { std::matches!(std::hint::black_box(1u8), 1) }"
                    .into(),
            };
            config
        };
        let limits = MetadataSnapshotLimits {
            max_crates: 1024,
            max_metadata_bytes: 512 * 1024 * 1024,
            max_mapping_entries: 1024 * 1024,
        };
        let source_limits = ExternalSourceSnapshotLimits {
            metadata: limits,
            max_files: 128 * 1024,
            max_declared_source_bytes: 1024 * 1024 * 1024,
        };
        for (cache_enabled, retire_by_budget) in [(false, false), (true, true), (true, false)] {
            with_resident_compiler_batch(make_config(), |batch, first| {
                with_resident_external_decoder_cache(cache_enabled, || {
                    let mut snapshots = Vec::new();
                    for (revision, config) in [first, make_config()].into_iter().enumerate() {
                        let snapshot = batch.run(config, |compiler, domain| {
                    let krate = crate::passes::parse(&compiler.sess);
                    let (snapshot, locks) = crate::passes::create_and_enter_global_ctxt_in_domain(
                        compiler,
                        domain,
                        krate,
                        |tcx| {
                            tcx.ensure_ok().analysis(());
                            // Demand an actual external allocation, not just a bundle
                            // pointer. OS is encoded as a slice in std metadata.
                            let std_crate = tcx.crates(()).iter().copied()
                                .find(|&cnum| tcx.crate_name(cnum) == rustc_span::sym::std)
                                .expect("loaded std");
                            let mut constant = std_crate.as_def_id();
                            for component in ["env", "consts", "OS"] {
                                constant = tcx.module_children(constant).iter()
                                    .find(|child| child.ident.name.as_str() == component)
                                    .and_then(|child| child.res.opt_def_id())
                                    .expect("std::env::consts::OS");
                            }
                            let rustc_middle::mir::ConstValue::Slice { alloc_id, meta } =
                                tcx.const_eval_poly(constant).expect("external OS constant")
                            else { panic!("OS must decode an actual slice allocation") };
                            let memory = tcx.global_alloc(alloc_id).unwrap_memory();
                            let bytes = memory.inner().get_bytes_unchecked(
                                rustc_middle::mir::interpret::alloc_range(
                                    rustc_abi::Size::ZERO, rustc_abi::Size::from_bytes(meta)));
                            assert_eq!(bytes, std::env::consts::OS.as_bytes());
                            assert!(
                                tcx.crates(())
                                    .iter()
                                    .any(|&cnum| { tcx.crate_name(cnum) == rustc_span::sym::std }),
                                "fixture must load actual standard-library metadata"
                            );
                            let snapshot = {
                                let cstore = CStore::from_tcx(tcx);
                                let snapshot = cstore
                                    .capture_external_metadata(limits)
                                    .expect("bounded actual loaded metadata snapshot");
                                assert_eq!(
                                    snapshot,
                                    cstore.capture_external_metadata(limits).unwrap(),
                                    "repeat capture must compare actual loaded bytes and maps"
                                );
                                for (bounded, error) in [
                                    (
                                        MetadataSnapshotLimits { max_crates: 0, ..limits },
                                        MetadataSnapshotError::CrateBudget,
                                    ),
                                    (
                                        MetadataSnapshotLimits { max_metadata_bytes: 0, ..limits },
                                        MetadataSnapshotError::MetadataBudget,
                                    ),
                                    (
                                        MetadataSnapshotLimits { max_mapping_entries: 0, ..limits },
                                        MetadataSnapshotError::MappingBudget,
                                    ),
                                ] {
                                    assert_eq!(
                                        cstore.capture_external_metadata(bounded),
                                        Err(error)
                                    );
                                }
                                assert_eq!(
                                    snapshot,
                                    cstore.capture_external_metadata(limits).unwrap(),
                                    "budget rejection must not change loaded compiler inputs"
                                );
                                snapshot
                            };
                            // Capture must inspect existing imports, never populate them.
                            if !cache_enabled || revision == 0 {
                                assert_eq!(
                                    CStore::from_tcx(tcx).capture_external_sources(source_limits),
                                    Err(ExternalSourceSnapshotError::IncompleteImports)
                                );
                            }
                            // Explicit test setup imports every metadata source. Do not hold
                            // a CStore guard across the hook, which acquires its own guard.
                            for &cnum in tcx.crates(()) {
                                tcx.import_source_files(cnum);
                            }
                            let sources = {
                                let cstore = CStore::from_tcx(tcx);
                                let sources = cstore
                                    .capture_external_sources(source_limits)
                                    .expect("complete actual imported source universe");
                                assert_eq!(
                                    sources,
                                    cstore.capture_external_sources(source_limits).unwrap()
                                );
                                for (bounded, error) in [
                                    (
                                        ExternalSourceSnapshotLimits {
                                            max_files: 0,
                                            ..source_limits
                                        },
                                        ExternalSourceSnapshotError::FileBudget,
                                    ),
                                    (
                                        ExternalSourceSnapshotLimits {
                                            max_declared_source_bytes: 0,
                                            ..source_limits
                                        },
                                        ExternalSourceSnapshotError::SourceByteBudget,
                                    ),
                                ] {
                                    assert_eq!(
                                        cstore.capture_external_sources(bounded),
                                        Err(error)
                                    );
                                }
                                assert_eq!(
                                    sources,
                                    cstore.capture_external_sources(source_limits).unwrap(),
                                    "budget rejection must not mutate imported sources"
                                );
                                sources
                            };
                            assert!(matches!(
                                build_revision_identity(
                                    tcx,
                                    FrozenSemanticInvocation {
                                        namespace: b"native-loaded-metadata-prerequisite-v1",
                                        options: b"fixed-std-fixture",
                                        body_revision: b"unchanged",
                                    },
                                    IdentityBudget {
                                        max_crates: 1024,
                                        max_definitions: 1024,
                                        max_identity_bytes: 1024 * 1024,
                                    },
                                ),
                                Err(CompilerIdentityError::UnsupportedExternalCrates)
                            ));
                            if cache_enabled {
                                let cstore = CStore::from_tcx(tcx);
                                cstore.publish_external_decoders(tcx, source_limits)
                                    .expect("bounded complete source and metadata publication");
                                let stats = rustc_metadata::resident_metadata::stats();
                                assert_eq!(stats.publications, revision + 1);
                                if revision == 0 {
                                    assert_eq!(stats.hits, 0);
                                } else {
                                    assert!(stats.hits > 0, "actual constructor must select retained decoder storage");
                                    if retire_by_budget {
                                        assert_eq!(cstore.publish_external_decoders(tcx,
                                            ExternalSourceSnapshotLimits { max_files: 0, ..source_limits }),
                                            Err(ExternalSourceSnapshotError::FileBudget));
                                    }
                                }
                            } else {
                                assert_eq!(rustc_metadata::resident_metadata::stats().hits, 0);
                            }
                            (snapshot, sources, tcx.allocation_universe_id(), alloc_id, memory)
                        },
                    )
                    .expect("available context storage");
                    assert!(locks.is_none());
                    snapshot
                });
                        snapshots.push(snapshot);
                    }
                    // These owned values survive their original contexts. Equal inputs do
                    // not authorize external DefId/source/interner or typed query reuse.
                    assert_eq!(snapshots[0].0, snapshots[1].0);
                    if cache_enabled {
                        assert_eq!(
                            snapshots[0].1, snapshots[1].1,
                            "explicit native cache must preserve actual imported source allocations"
                        );
                        assert_eq!(snapshots[0].2, snapshots[1].2);
                        assert_eq!(
                            snapshots[0].3, snapshots[1].3,
                            "retained decoder must preserve actual external allocation IDs"
                        );
                        assert!(std::ptr::eq(snapshots[0].4.inner(), snapshots[1].4.inner()));
                        let refusal = rustc_errors::catch_fatal_errors(|| {
                            batch.run(make_config(), |compiler, domain| {
                                let krate = crate::passes::parse(&compiler.sess);
                                if retire_by_budget {
                                    crate::passes::create_and_enter_global_ctxt_in_domain(
                                        compiler,
                                        domain,
                                        krate,
                                        |tcx| {
                                            tcx.ensure_ok().analysis(());
                                        },
                                    )
                                    .expect("available context storage");
                                } else {
                                    // Same SessionGlobals and SourceMap, identical std bytes,
                                    // but an independent AllocMap. Reject before decoding an
                                    // old allocation ID into that fresh map.
                                    crate::passes::create_and_enter_global_ctxt(
                                        compiler,
                                        krate,
                                        |tcx| {
                                            assert_ne!(
                                                snapshots[0].2,
                                                tcx.allocation_universe_id()
                                            );
                                            tcx.ensure_ok().analysis(());
                                        },
                                    );
                                }
                            })
                        });
                        assert!(
                            refusal.is_err(),
                            "publication failure or allocation-universe change must retire the owner, not decode stale IDs"
                        );
                        assert!(rustc_metadata::resident_metadata::stats().retired);
                    } else {
                        assert_eq!(snapshots[0].2, snapshots[1].2);
                        assert_ne!(
                            snapshots[0].3, snapshots[1].3,
                            "disabled decoder cache must decode a fresh allocation ID"
                        );
                        assert_ne!(
                            snapshots[0].1, snapshots[1].1,
                            "equal external metadata alone does not imply shared source allocations"
                        );
                    }
                });
            });
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
