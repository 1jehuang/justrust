//! Exact, bounded external metadata inputs for resident-compiler identity work.
//!
//! Equality here is only an input prerequisite. It does not establish that external
//! DefIds, source positions, hygiene, or interned values can cross compiler contexts.
//! In particular this API must not be used to bypass resident reuse rejection.

use crate::creader::CStore;
use rustc_data_structures::sync::Lock;
use rustc_middle::ty::TyCtxt;
use rustc_span::def_id::CrateNum;
use rustc_span::hygiene::HygieneDecodeContext;
use rustc_span::source_map::SourceMap;
use rustc_span::{ExternalSource, ExternalSourceKind, Pos, SourceFile};
use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Default)]
pub struct ExternalDecoderCacheStats {
    pub hits: usize,
    pub misses: usize,
    pub rejected: usize,
    pub publications: usize,
    pub retained_entries: usize,
    pub retired: bool,
    pub retirement_requests: usize,
}

#[derive(PartialEq, Eq)]
struct TranslationInputs {
    real_source: Option<PathBuf>,
    real_compiler: Option<PathBuf>,
    simulated: Option<PathBuf>,
    translate: bool,
    remaps: Vec<(PathBuf, PathBuf)>,
}

impl TranslationInputs {
    fn matches(&self, tcx: TyCtxt<'_>) -> bool {
        let opts = &tcx.sess.opts;
        self.real_source == opts.real_rust_source_base_dir
            && self.real_compiler == opts.real_rustc_dev_source_base_dir
            && self.simulated == opts.unstable_opts.simulate_remapped_rust_src_base
            && self.translate == opts.unstable_opts.translate_remapped_path_to_local_path
            && self.remaps == opts.remap_path_prefix
    }

    fn capture(tcx: TyCtxt<'_>, budget: usize) -> Result<Self, ExternalSourceSnapshotError> {
        fn copy_path(path: &std::path::Path) -> Result<PathBuf, ExternalSourceSnapshotError> {
            let mut copy = std::ffi::OsString::new();
            copy.try_reserve_exact(path.as_os_str().len())
                .map_err(|_| ExternalSourceSnapshotError::Allocation)?;
            copy.push(path);
            Ok(copy.into())
        }
        let opts = &tcx.sess.opts;
        let paths = opts
            .real_rust_source_base_dir
            .iter()
            .chain(opts.real_rustc_dev_source_base_dir.iter())
            .chain(opts.unstable_opts.simulate_remapped_rust_src_base.iter())
            .chain(opts.remap_path_prefix.iter().flat_map(|(a, b)| [a, b]));
        let mut bytes = 0usize;
        for path in paths {
            bytes = bytes
                .checked_add(path.as_os_str().len())
                .ok_or(ExternalSourceSnapshotError::SourceByteBudget)?;
            if bytes > budget {
                return Err(ExternalSourceSnapshotError::SourceByteBudget);
            }
        }
        if opts.remap_path_prefix.len() > budget / std::mem::size_of::<(PathBuf, PathBuf)>() {
            return Err(ExternalSourceSnapshotError::SourceByteBudget);
        }
        let mut remaps = Vec::new();
        remaps
            .try_reserve_exact(opts.remap_path_prefix.len())
            .map_err(|_| ExternalSourceSnapshotError::Allocation)?;
        for (from, to) in &opts.remap_path_prefix {
            remaps.push((copy_path(from)?, copy_path(to)?));
        }
        Ok(Self {
            real_source: opts.real_rust_source_base_dir.as_deref().map(copy_path).transpose()?,
            real_compiler: opts
                .real_rustc_dev_source_base_dir
                .as_deref()
                .map(copy_path)
                .transpose()?,
            simulated: opts
                .unstable_opts
                .simulate_remapped_rust_src_base
                .as_deref()
                .map(copy_path)
                .transpose()?,
            translate: opts.unstable_opts.translate_remapped_path_to_local_path,
            remaps,
        })
    }
}

struct PublishedDecoders {
    snapshot: ExternalSourceSnapshot,
    source_map: Arc<SourceMap>,
    translation: TranslationInputs,
    states: Vec<(u32, Arc<ExternalDecoderIdentityState>)>,
}

#[derive(Default)]
struct DecoderCache {
    published: Option<PublishedDecoders>,
    stats: ExternalDecoderCacheStats,
}

impl DecoderCache {
    fn retire(&mut self) {
        self.stats.retired = true;
        self.stats.retirement_requests += 1;
    }

    fn publish(
        &mut self,
        capture: impl FnOnce() -> Result<PublishedDecoders, ExternalSourceSnapshotError>,
    ) -> Result<(), ExternalSourceSnapshotError> {
        self.published = None;
        self.stats.retained_entries = 0;
        if self.stats.retired {
            return Err(ExternalSourceSnapshotError::Retired);
        }
        let published = match capture() {
            Ok(published) => published,
            Err(error) => {
                self.retire();
                return Err(error);
            }
        };
        self.stats.retained_entries = published.states.len();
        self.stats.publications += 1;
        self.published = Some(published);
        Ok(())
    }
}

thread_local! {
    static DECODERS: RefCell<Option<DecoderCache>> = const { RefCell::new(None) };
}

/// The caller must enter this scope before the first external metadata decode
/// in a fresh SessionGlobals universe, keep that universe alive, and use this
/// same serial thread throughout the closure. No state survives scope exit or
/// unwind. Nested enabled or disabled scopes inside an active owner are rejected:
/// compiling with a masked owner could contaminate global foreign expansion IDs.
pub fn with_resident_external_decoder_cache<R>(enabled: bool, f: impl FnOnce() -> R) -> R {
    let previous = DECODERS.with(|slot| {
        let mut slot = slot.borrow_mut();
        assert!(slot.is_none(), "resident external decoder scopes cannot nest");
        std::mem::replace(&mut *slot, enabled.then(DecoderCache::default))
    });
    struct Reset(Option<DecoderCache>);
    impl Drop for Reset {
        fn drop(&mut self) {
            DECODERS.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let _reset = Reset(previous);
    f()
}

pub fn stats() -> ExternalDecoderCacheStats {
    DECODERS.with(|slot| slot.borrow().as_ref().map(|c| c.stats).unwrap_or_default())
}

fn absent_text(source: &SourceFile) -> bool {
    matches!(
        &*source.external_src.read(),
        ExternalSource::Foreign { kind: ExternalSourceKind::AbsentOk, .. }
    )
}

fn mapped_targets_loaded(
    current: u32,
    mut map: impl Iterator<Item = u32>,
    is_loaded: impl Fn(u32) -> bool,
) -> bool {
    map.all(|target| target == current || is_loaded(target))
}

impl PublishedDecoders {
    fn candidate(
        &self,
        cnum: u32,
        bytes: &[u8],
        map: impl Iterator<Item = u32>,
        source_map: &Arc<SourceMap>,
        translation_matches: bool,
        proc_macro: bool,
    ) -> Option<Arc<ExternalDecoderIdentityState>> {
        if proc_macro
            || !Arc::ptr_eq(&self.source_map, source_map)
            || !translation_matches
            || !self.entry_matches(cnum, bytes, map)
            || self.snapshot.files.iter().any(|file| !absent_text(&file.source))
        {
            return None;
        }
        self.state(cnum).cloned()
    }

    fn entry_matches(&self, cnum: u32, bytes: &[u8], map: impl Iterator<Item = u32>) -> bool {
        self.snapshot.metadata.crates.iter().find(|entry| entry.current_crate == cnum).is_some_and(
            |entry| entry.metadata == bytes && entry.crate_num_map.iter().copied().eq(map),
        )
    }

    fn state(&self, cnum: u32) -> Option<&Arc<ExternalDecoderIdentityState>> {
        self.states.iter().find(|(n, _)| *n == cnum).map(|(_, state)| state)
    }

    fn loaded_matches(
        &self,
        cnum: u32,
        bytes: &[u8],
        map: impl Iterator<Item = u32>,
        state: &Arc<ExternalDecoderIdentityState>,
        proc_macro: bool,
    ) -> bool {
        !proc_macro
            && self.entry_matches(cnum, bytes, map)
            && self.state(cnum).is_some_and(|retained| Arc::ptr_eq(retained, state))
    }
}

impl CStore {
    /// Publish only complete, already-imported source/hygiene storage. Failure
    /// invalidates the previous universe rather than retaining a stale fallback.
    pub fn publish_external_decoders(
        &self,
        tcx: TyCtxt<'_>,
        limits: ExternalSourceSnapshotLimits,
    ) -> Result<(), ExternalSourceSnapshotError> {
        DECODERS.with(|slot| {
            let mut slot = slot.borrow_mut();
            let cache = slot.as_mut().ok_or(ExternalSourceSnapshotError::Disabled)?;
            cache.publish(|| {
                let snapshot = self.capture_external_sources(limits)?;
                if snapshot.files.iter().any(|file| !absent_text(&file.source)) {
                    return Err(ExternalSourceSnapshotError::CachedSourceText);
                }
                let translation =
                    TranslationInputs::capture(tcx, limits.max_declared_source_bytes)?;
                let mut states = Vec::new();
                states
                    .try_reserve_exact(snapshot.metadata.crates.len())
                    .map_err(|_| ExternalSourceSnapshotError::Allocation)?;
                for (cnum, data) in self.iter_crate_data() {
                    states.push((cnum.as_u32(), data.resident_decoder_identity()));
                }
                Ok(PublishedDecoders {
                    snapshot,
                    source_map: tcx.sess.psess.clone_source_map(),
                    translation,
                    states,
                })
            })
        })
    }

    pub(crate) fn select_external_decoder(
        &self,
        tcx: TyCtxt<'_>,
        cnum: CrateNum,
        bytes: &[u8],
        map: &[CrateNum],
        proc_macro: bool,
    ) -> Arc<ExternalDecoderIdentityState> {
        let selected = DECODERS.with(|slot| {
            let mut slot = slot.borrow_mut();
            let cache = slot.as_mut()?;
            if cache.stats.retired {
                return None;
            }
            let selected = cache.published.as_ref().and_then(|published| {
                let candidate = published.candidate(
                    cnum.as_u32(),
                    bytes,
                    map.iter().map(|n| n.as_u32()),
                    &tcx.sess.psess.clone_source_map(),
                    published.translation.matches(tcx),
                    proc_macro,
                )?;
                for (loaded, data) in self.iter_crate_data() {
                    let (bytes, map) = data.resident_metadata_inputs();
                    if !published.loaded_matches(
                        loaded.as_u32(),
                        bytes,
                        map.iter().map(|n| n.as_u32()),
                        &data.resident_decoder_identity(),
                        data.is_proc_macro_crate(),
                    ) {
                        return None;
                    }
                }
                // Self is not inserted yet. Every other mapped target must be
                // actually loaded, not merely present in the retained universe.
                if !mapped_targets_loaded(cnum.as_u32(), map.iter().map(|n| n.as_u32()), |dep| {
                    self.iter_crate_data().any(|(loaded, _)| loaded.as_u32() == dep)
                }) {
                    return None;
                }
                let mut sources = Vec::new();
                sources.try_reserve_exact(published.snapshot.files.len()).ok()?;
                for file in &published.snapshot.files {
                    if file.current_crate == cnum.as_u32() {
                        sources.push(Arc::clone(&file.source));
                    }
                }
                tcx.sess.source_map().activate_resident_imported_sources(&sources).ok()?;
                Some(candidate)
            });
            if selected.is_some() {
                cache.stats.hits += 1;
            } else {
                cache.stats.misses += 1;
                if cache.published.is_some() {
                    cache.stats.rejected += 1;
                    cache.retire();
                }
            }
            selected
        });
        // Global foreign expansion data is keyed by numeric CrateNum/ExpnIndex,
        // independently of this decoder bundle. A fresh bundle cannot sanitize
        // that universe after a rejection. Only dropping SessionGlobals can.
        if stats().retired {
            tcx.dcx().fatal("resident external decoder identity changed; retire the SessionGlobals owner before retrying");
        }
        selected.unwrap_or_else(|| Arc::new(ExternalDecoderIdentityState::default()))
    }
}

/// The actual per-metadata decoder import relation. Shared here so capture can
/// inspect it without exposing the private decoder module or forcing an import.
#[derive(Clone)]
pub(crate) struct ImportedSourceFile {
    pub(crate) original_start_pos: rustc_span::BytePos,
    pub(crate) original_end_pos: rustc_span::BytePos,
    pub(crate) translated_source_file: Arc<SourceFile>,
}

/// Native identity-bearing decoder storage. Fresh by default, or selected from
/// one explicit scoped owner only after exact metadata/source validation.
#[derive(Default)]
pub(crate) struct ExternalDecoderIdentityState {
    pub(crate) source_map_import_info: Lock<Vec<Option<ImportedSourceFile>>>,
    /// SyntaxContext IDs are crate-local, so their decoding cache must remain
    /// paired with the actual source translations for that crate metadata.
    pub(crate) hygiene_context: HygieneDecodeContext,
}

/// Limits apply across all loaded crates. Source bytes are declared lengths,
/// not just bytes currently materialized in the source cache.
#[derive(Clone, Copy, Debug)]
pub struct ExternalSourceSnapshotLimits {
    /// Existing exact metadata prerequisite and its aggregate allocation limits.
    pub metadata: MetadataSnapshotLimits,
    /// Total source-map entries across all loaded crates, including transitive ones.
    pub max_files: usize,
    /// Sum of max(normalized length, unnormalized length) for every source file.
    pub max_declared_source_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExternalSourceSnapshotError {
    Disabled,
    CachedSourceText,
    Retired,
    Metadata(MetadataSnapshotError),
    IncompleteImports,
    UnsupportedMapping,
    FileBudget,
    SourceByteBudget,
    Allocation,
}

/// A prerequisite only, never permission to reuse compiler state. Retaining the
/// actual translated Arcs is essential: stable IDs, paths and content hashes do
/// not identify a source-map allocation or its numeric translation.
#[derive(Debug, PartialEq, Eq)]
pub struct ExternalSourceSnapshot {
    metadata: ExternalMetadataSnapshot,
    files: Vec<ExternalSourceMapping>,
}

#[derive(Debug)]
struct ExternalSourceMapping {
    current_crate: u32,
    metadata_index: u32,
    original_start: u32,
    original_end: u32,
    current_start: u32,
    current_end: u32,
    declared_bytes: u32,
    source: Arc<SourceFile>,
}

impl PartialEq for ExternalSourceMapping {
    fn eq(&self, other: &Self) -> bool {
        self.current_crate == other.current_crate
            && self.metadata_index == other.metadata_index
            && self.original_start == other.original_start
            && self.original_end == other.original_end
            && self.current_start == other.current_start
            && self.current_end == other.current_end
            && self.declared_bytes == other.declared_bytes
            && Arc::ptr_eq(&self.source, &other.source)
    }
}

impl Eq for ExternalSourceMapping {}

/// Validate the complete map and all totals before any source Arc is retained.
fn validate_source_inputs(
    current_crate: u32,
    expected: usize,
    imports: &[Option<ImportedSourceFile>],
    limits: ExternalSourceSnapshotLimits,
    files: &mut usize,
    bytes: &mut usize,
) -> Result<(), ExternalSourceSnapshotError> {
    use ExternalSourceSnapshotError::*;
    if imports.len() != expected || imports.iter().any(Option::is_none) {
        return Err(IncompleteImports);
    }
    let new_files = files.checked_add(expected).ok_or(FileBudget)?;
    if new_files > limits.max_files {
        return Err(FileBudget);
    }
    let mut new_bytes = *bytes;
    for (index, import) in imports.iter().enumerate() {
        let import = import.as_ref().ok_or(IncompleteImports)?;
        let source = &import.translated_source_file;
        let len = source.normalized_source_len.to_u32();
        let start = import.original_start_pos.0;
        let end = import.original_end_pos.0;
        if current_crate == 0
            || source.cnum.as_u32() != current_crate
            || u32::try_from(index).is_err()
            || start.checked_add(len) != Some(end)
            || source.start_pos.0.checked_add(len).is_none()
            || !matches!(
                &*source.external_src.read(),
                ExternalSource::Foreign { metadata_index, .. } if *metadata_index as usize == index
            )
        {
            return Err(UnsupportedMapping);
        }
        let declared = source.unnormalized_source_len.max(len) as usize;
        new_bytes = new_bytes.checked_add(declared).ok_or(SourceByteBudget)?;
        if new_bytes > limits.max_declared_source_bytes {
            return Err(SourceByteBudget);
        }
    }
    *files = new_files;
    *bytes = new_bytes;
    Ok(())
}

/// Caller-selected hard limits. All limits include every loaded external crate,
/// including transitive dependencies. No partial snapshot is returned on failure.
#[derive(Clone, Copy, Debug)]
pub struct MetadataSnapshotLimits {
    pub max_crates: usize,
    pub max_metadata_bytes: usize,
    pub max_mapping_entries: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetadataSnapshotError {
    ProcMacro,
    CrateBudget,
    MetadataBudget,
    MappingBudget,
    UnsupportedMapping,
    Allocation,
}

/// Owned inputs, with no references into the old compiler or its mapped artifacts.
/// Fields are private so a successful capture cannot be confused with a caller's
/// incomplete reconstruction. Equality compares bytes, not hashes or file paths.
#[derive(Debug, PartialEq, Eq)]
pub struct ExternalMetadataSnapshot {
    crates: Vec<ExternalCrateMetadata>,
}

#[derive(Debug, PartialEq, Eq)]
struct ExternalCrateMetadata {
    current_crate: u32,
    metadata: Vec<u8>,
    /// Index is the encoded crate number, value is this session's crate number.
    crate_num_map: Vec<u32>,
}

impl CStore {
    /// Capture only already imported source mappings, paired with the exact
    /// loaded metadata snapshot. No queries, decoding, or forced imports occur.
    /// An incompletely imported crate is rejected rather than silently omitted.
    pub fn capture_external_sources(
        &self,
        limits: ExternalSourceSnapshotLimits,
    ) -> Result<ExternalSourceSnapshot, ExternalSourceSnapshotError> {
        use ExternalSourceSnapshotError::*;
        let metadata = self.capture_external_metadata(limits.metadata).map_err(Metadata)?;
        let mut count = 0;
        let mut bytes = 0;
        for (cnum, data) in self.iter_crate_data() {
            data.with_resident_source_inputs(|expected, imports| {
                validate_source_inputs(
                    cnum.as_u32(),
                    expected,
                    imports,
                    limits,
                    &mut count,
                    &mut bytes,
                )
            })?;
        }
        let mut files = Vec::new();
        files.try_reserve_exact(count).map_err(|_| Allocation)?;
        let mut retained_count = 0;
        let mut retained_bytes = 0;
        for (cnum, data) in self.iter_crate_data() {
            data.with_resident_source_inputs(|expected, imports| {
                // Recheck while retaining under the same lock. Even malformed
                // concurrent decoding must not grow the bounded allocation.
                validate_source_inputs(
                    cnum.as_u32(),
                    expected,
                    imports,
                    limits,
                    &mut retained_count,
                    &mut retained_bytes,
                )?;
                if retained_count > count {
                    return Err(IncompleteImports);
                }
                for (index, import) in imports.iter().enumerate() {
                    let import = import.as_ref().ok_or(IncompleteImports)?;
                    let source = &import.translated_source_file;
                    files.push(ExternalSourceMapping {
                        current_crate: cnum.as_u32(),
                        metadata_index: index as u32,
                        original_start: import.original_start_pos.0,
                        original_end: import.original_end_pos.0,
                        current_start: source.start_pos.0,
                        current_end: source.start_pos.0 + source.normalized_source_len.to_u32(),
                        declared_bytes: source.unnormalized_source_len,
                        source: Arc::clone(source),
                    });
                }
                Ok(())
            })?;
        }
        Ok(ExternalSourceSnapshot { metadata, files })
    }

    /// Copy the bytes actually held by each loaded `CrateMetadata`, never reopen
    /// an artifact. The caller must capture after loading is complete and compare
    /// again if loading can occur later. Proc macros require additional executable
    /// identity and state support, so they are deliberately rejected.
    pub fn capture_external_metadata(
        &self,
        limits: MetadataSnapshotLimits,
    ) -> Result<ExternalMetadataSnapshot, MetadataSnapshotError> {
        let mut builder = SnapshotBuilder::new(limits);
        for (cnum, data) in self.iter_crate_data() {
            if data.is_proc_macro_crate() {
                return Err(MetadataSnapshotError::ProcMacro);
            }
            let (bytes, map) = data.resident_metadata_inputs();
            builder.push(cnum.as_u32(), bytes, map.iter().map(|n| n.as_u32()))?;
        }
        builder.finish()
    }
}

struct SnapshotBuilder {
    limits: MetadataSnapshotLimits,
    bytes: usize,
    mappings: usize,
    crates: Vec<ExternalCrateMetadata>,
}

impl SnapshotBuilder {
    fn new(limits: MetadataSnapshotLimits) -> Self {
        Self { limits, bytes: 0, mappings: 0, crates: Vec::new() }
    }

    fn push(
        &mut self,
        current_crate: u32,
        bytes: &[u8],
        map: impl ExactSizeIterator<Item = u32>,
    ) -> Result<(), MetadataSnapshotError> {
        use MetadataSnapshotError::*;
        if self.crates.len() >= self.limits.max_crates {
            return Err(CrateBudget);
        }
        let new_bytes = self.bytes.checked_add(bytes.len()).ok_or(MetadataBudget)?;
        if new_bytes > self.limits.max_metadata_bytes {
            return Err(MetadataBudget);
        }
        let new_mappings = self.mappings.checked_add(map.len()).ok_or(MappingBudget)?;
        if new_mappings > self.limits.max_mapping_entries {
            return Err(MappingBudget);
        }
        if current_crate == 0 || bytes.is_empty() {
            return Err(UnsupportedMapping);
        }
        let mut metadata = Vec::new();
        metadata.try_reserve_exact(bytes.len()).map_err(|_| Allocation)?;
        metadata.extend_from_slice(bytes);
        let mut crate_num_map = Vec::new();
        crate_num_map.try_reserve_exact(map.len()).map_err(|_| Allocation)?;
        crate_num_map.extend(map);
        // The encoded local crate must map back to this external crate.
        if crate_num_map.first() != Some(&current_crate) {
            return Err(UnsupportedMapping);
        }
        self.crates.try_reserve(1).map_err(|_| Allocation)?;
        self.crates.push(ExternalCrateMetadata { current_crate, metadata, crate_num_map });
        self.bytes = new_bytes;
        self.mappings = new_mappings;
        Ok(())
    }

    fn finish(self) -> Result<ExternalMetadataSnapshot, MetadataSnapshotError> {
        // Every mapped target must be present, and crate numbers must be unique
        // and ordered as in CStore. Reject holes referenced by a mapping.
        if self.crates.windows(2).any(|w| w[0].current_crate >= w[1].current_crate)
            || self.crates.iter().any(|c| {
                c.crate_num_map
                    .iter()
                    .any(|n| self.crates.binary_search_by_key(n, |c| c.current_crate).is_err())
            })
        {
            return Err(MetadataSnapshotError::UnsupportedMapping);
        }
        Ok(ExternalMetadataSnapshot { crates: self.crates })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustc_data_structures::sync::FreezeLock;
    use rustc_span::def_id::CrateNum;
    use rustc_span::{BytePos, ExternalSourceKind, FileName, SourceFileHashAlgorithm};

    fn published() -> PublishedDecoders {
        let import = imported(0);
        let state = Arc::new(ExternalDecoderIdentityState::default());
        state.source_map_import_info.lock().push(Some(import.clone()));
        PublishedDecoders {
            snapshot: ExternalSourceSnapshot {
                metadata: snapshot(&[1, 2, 3], &[1]),
                files: vec![mapping(&import)],
            },
            source_map: Arc::new(SourceMap::new(rustc_span::source_map::FilePathMapping::empty())),
            translation: TranslationInputs {
                real_source: None,
                real_compiler: None,
                simulated: None,
                translate: true,
                remaps: Vec::new(),
            },
            states: vec![(1, state)],
        }
    }

    #[test]
    fn candidate_selects_actual_native_owner_only_for_exact_inputs() {
        let p = published();
        let select =
            |cnum, bytes: &[u8], map: &[u32], sm: &Arc<SourceMap>, translation, proc_macro| {
                p.candidate(cnum, bytes, map.iter().copied(), sm, translation, proc_macro)
            };
        let selected = select(1, &[1, 2, 3], &[1], &p.source_map, true, false).unwrap();
        assert!(Arc::ptr_eq(&selected, p.state(1).unwrap()));
        assert!(select(2, &[1, 2, 3], &[1], &p.source_map, true, false).is_none());
        assert!(select(1, &[1, 2, 4], &[1], &p.source_map, true, false).is_none());
        assert!(select(1, &[1, 2, 3], &[1, 1], &p.source_map, true, false).is_none());
        assert!(select(1, &[1, 2, 3], &[2], &p.source_map, true, false).is_none());
        assert!(select(1, &[1, 2, 3], &[1], &p.source_map, false, false).is_none());
        assert!(select(1, &[1, 2, 3], &[1], &p.source_map, true, true).is_none());
        let other_map = Arc::new(SourceMap::new(rustc_span::source_map::FilePathMapping::empty()));
        assert!(select(1, &[1, 2, 3], &[1], &other_map, true, false).is_none());
    }

    #[test]
    fn loaded_dependency_requires_exact_bytes_map_and_actual_owner() {
        let p = published();
        let owner = p.state(1).unwrap();
        assert!(p.loaded_matches(1, &[1, 2, 3], [1].into_iter(), owner, false));
        assert!(!p.loaded_matches(1, &[1, 2, 4], [1].into_iter(), owner, false));
        assert!(!p.loaded_matches(1, &[1, 2, 3], [1, 1].into_iter(), owner, false));
        assert!(!p.loaded_matches(2, &[1, 2, 3], [1].into_iter(), owner, false));
        assert!(!p.loaded_matches(1, &[1, 2, 3], [1].into_iter(), owner, true));
        let fresh = Arc::new(ExternalDecoderIdentityState::default());
        assert!(!p.loaded_matches(1, &[1, 2, 3], [1].into_iter(), &fresh, false));
    }

    #[test]
    fn retained_universe_does_not_substitute_for_actually_loaded_dependencies() {
        assert!(mapped_targets_loaded(1, [1].into_iter(), |_| false));
        assert!(!mapped_targets_loaded(1, [1, 2].into_iter(), |_| false));
        assert!(mapped_targets_loaded(1, [1, 2].into_iter(), |n| n == 2));
        assert!(!mapped_targets_loaded(1, [1, 2, 3].into_iter(), |n| n == 2));
    }

    #[test]
    fn any_retained_foreign_text_cache_change_rejects_selection() {
        for kind in
            [ExternalSourceKind::AbsentErr, ExternalSourceKind::Present(Arc::new("abc".into()))]
        {
            let mut p = published();
            // An additional retained file must also remain AbsentOk, even when
            // it is not the source selected by this candidate's metadata entry.
            let mut source = imported(0);
            Arc::get_mut(&mut source.translated_source_file).unwrap().external_src =
                FreezeLock::new(ExternalSource::Foreign { kind, metadata_index: 0 });
            p.snapshot.files.push(mapping(&source));
            assert!(
                p.candidate(1, &[1, 2, 3], [1].into_iter(), &p.source_map, true, false).is_none()
            );
        }
    }

    #[test]
    fn failed_publication_clears_previous_native_owner_before_capture() {
        let mut cache = DecoderCache::default();
        cache.publish(|| Ok(published())).unwrap();
        let old = Arc::downgrade(cache.published.as_ref().unwrap().state(1).unwrap());
        assert_eq!(cache.stats.retained_entries, 1);
        assert_eq!(
            cache.publish(|| {
                assert!(old.upgrade().is_none());
                Err(ExternalSourceSnapshotError::IncompleteImports)
            }),
            Err(ExternalSourceSnapshotError::IncompleteImports)
        );
        assert!(cache.published.is_none());
        assert_eq!(cache.stats.retained_entries, 0);
        assert_eq!(cache.stats.publications, 1);
        assert!(cache.stats.retired);
        assert_eq!(cache.stats.retirement_requests, 1);
        assert_eq!(
            cache.publish(|| panic!("retired owner must not capture again")),
            Err(ExternalSourceSnapshotError::Retired)
        );
        assert!(cache.stats.retired);
    }

    #[test]
    fn even_initial_capture_failure_retires_the_owner() {
        let mut cache = DecoderCache::default();
        assert_eq!(
            cache.publish(|| Err(ExternalSourceSnapshotError::IncompleteImports)),
            Err(ExternalSourceSnapshotError::IncompleteImports)
        );
        assert!(cache.stats.retired);
        assert_eq!(cache.stats.publications, 0);
        assert_eq!(cache.stats.retirement_requests, 1);
    }

    #[test]
    fn decoder_owner_is_default_off_and_dropped_on_unwind() {
        assert!(DECODERS.with(|slot| slot.borrow().is_none()));
        let result = std::panic::catch_unwind(|| {
            with_resident_external_decoder_cache(true, || {
                DECODERS
                    .with(|slot| slot.borrow_mut().as_mut().unwrap().published = Some(published()));
                panic!("test owner unwind");
            });
        });
        assert!(result.is_err());
        assert!(DECODERS.with(|slot| slot.borrow().is_none()));
        assert_eq!(stats().retained_entries, 0);
        assert!(!stats().retired);
    }

    #[test]
    fn decoder_scopes_reject_enabled_and_disabled_nesting() {
        with_resident_external_decoder_cache(true, || {
            DECODERS.with(|slot| slot.borrow_mut().as_mut().unwrap().stats.hits = 7);
            for enabled in [false, true] {
                assert!(
                    std::panic::catch_unwind(|| {
                        with_resident_external_decoder_cache(enabled, || panic!("must not enter"));
                    })
                    .is_err()
                );
                assert_eq!(stats().hits, 7);
            }
        });
        assert!(DECODERS.with(|slot| slot.borrow().is_none()));
        with_resident_external_decoder_cache(false, || assert_eq!(stats().hits, 0));
    }

    fn source_limits() -> ExternalSourceSnapshotLimits {
        ExternalSourceSnapshotLimits {
            metadata: limits(),
            max_files: 2,
            max_declared_source_bytes: 6,
        }
    }

    fn imported(index: u32) -> ImportedSourceFile {
        let mut source = SourceFile::new(
            FileName::Custom("resident-external-source".into()),
            "abc".into(),
            SourceFileHashAlgorithm::Md5,
            None,
        )
        .unwrap();
        source.cnum = CrateNum::from_u32(1);
        source.start_pos = BytePos(100 + index * 4);
        source.external_src = FreezeLock::new(ExternalSource::Foreign {
            kind: ExternalSourceKind::AbsentOk,
            metadata_index: index,
        });
        ImportedSourceFile {
            original_start_pos: BytePos(10 + index * 4),
            original_end_pos: BytePos(13 + index * 4),
            translated_source_file: Arc::new(source),
        }
    }

    fn mapping(import: &ImportedSourceFile) -> ExternalSourceMapping {
        ExternalSourceMapping {
            current_crate: 1,
            metadata_index: 0,
            original_start: import.original_start_pos.0,
            original_end: import.original_end_pos.0,
            current_start: import.translated_source_file.start_pos.0,
            current_end: import.translated_source_file.end_position().0,
            declared_bytes: import.translated_source_file.unnormalized_source_len,
            source: Arc::clone(&import.translated_source_file),
        }
    }

    #[test]
    fn fresh_decoder_bundles_have_independent_native_storage() {
        let first = Arc::new(ExternalDecoderIdentityState::default());
        let second = Arc::new(ExternalDecoderIdentityState::default());
        assert!(!Arc::ptr_eq(&first, &second));
        assert!(!std::ptr::eq(&first.hygiene_context, &second.hygiene_context));
        assert!(!std::ptr::eq(&first.source_map_import_info, &second.source_map_import_info));
        first.source_map_import_info.lock().push(Some(imported(0)));
        second.source_map_import_info.lock().push(Some(imported(0)));
        let first_imports = first.source_map_import_info.lock();
        let second_imports = second.source_map_import_info.lock();
        assert_ne!(first_imports.as_ptr(), second_imports.as_ptr());
        assert_ne!(
            mapping(first_imports[0].as_ref().unwrap()),
            mapping(second_imports[0].as_ref().unwrap())
        );
    }

    #[test]
    fn cloned_decoder_bundle_preserves_actual_imports_and_hygiene_identity() {
        let original = Arc::new(ExternalDecoderIdentityState::default());
        original.source_map_import_info.lock().push(Some(imported(0)));
        let cloned = Arc::clone(&original);
        assert!(Arc::ptr_eq(&original, &cloned));
        assert!(std::ptr::eq(&original.hygiene_context, &cloned.hygiene_context));
        assert!(std::ptr::eq(&original.source_map_import_info, &cloned.source_map_import_info));
        let (entry_ptr, source) = {
            let imports = original.source_map_import_info.lock();
            (imports.as_ptr(), imports[0].as_ref().unwrap().clone())
        };
        {
            let imports = cloned.source_map_import_info.lock();
            assert_eq!(entry_ptr, imports.as_ptr());
            assert_eq!(mapping(&source), mapping(imports[0].as_ref().unwrap()));
            assert!(Arc::ptr_eq(
                &source.translated_source_file,
                &imports[0].as_ref().unwrap().translated_source_file
            ));
        }
        cloned.source_map_import_info.lock().push(None);
        assert_eq!(original.source_map_import_info.lock().len(), 2);
    }

    #[test]
    fn source_identity_requires_the_actual_arc_and_original_translation() {
        let source = imported(0);
        let cloned = source.clone();
        assert_eq!(mapping(&source), mapping(&cloned));
        // All metadata and source content are identical, but allocations differ.
        assert_ne!(mapping(&source), mapping(&imported(0)));
        let mut shifted = source.clone();
        shifted.original_start_pos.0 += 1;
        shifted.original_end_pos.0 += 1;
        assert_ne!(mapping(&source), mapping(&shifted));
        let mut changed_index = mapping(&source);
        changed_index.metadata_index = 1;
        assert_ne!(mapping(&source), changed_index);
    }

    #[test]
    fn source_coverage_is_complete_and_bounded_before_retention() {
        use ExternalSourceSnapshotError::*;
        let imports = [Some(imported(0)), Some(imported(1))];
        let (mut files, mut bytes) = (0, 0);
        let limits = source_limits();
        validate_source_inputs(1, 2, &imports, limits, &mut files, &mut bytes).unwrap();
        assert_eq!((files, bytes), (2, 6));
        assert_eq!(Arc::strong_count(&imports[0].as_ref().unwrap().translated_source_file), 1);
        assert_eq!(
            validate_source_inputs(1, 1, &imports[..1], limits, &mut files, &mut bytes),
            Err(FileBudget)
        );
        assert_eq!((files, bytes), (2, 6));
        let (mut files, mut bytes) = (0, 0);
        assert_eq!(
            validate_source_inputs(1, 2, &imports[..1], limits, &mut files, &mut bytes),
            Err(IncompleteImports)
        );
        assert_eq!(
            validate_source_inputs(
                1,
                2,
                &[Some(imported(0)), None],
                limits,
                &mut files,
                &mut bytes
            ),
            Err(IncompleteImports)
        );
        let limits = ExternalSourceSnapshotLimits { max_declared_source_bytes: 5, ..limits };
        assert_eq!(
            validate_source_inputs(1, 2, &imports, limits, &mut files, &mut bytes),
            Err(SourceByteBudget)
        );
        assert_eq!((files, bytes), (0, 0));
    }

    #[test]
    fn source_owner_index_and_positions_must_match_the_import_cache() {
        use ExternalSourceSnapshotError::UnsupportedMapping;
        let check =
            |source| validate_source_inputs(1, 1, &[Some(source)], source_limits(), &mut 0, &mut 0);
        assert_eq!(check(imported(1)), Err(UnsupportedMapping));
        let mut wrong_owner = imported(0);
        Arc::get_mut(&mut wrong_owner.translated_source_file).unwrap().cnum = CrateNum::from_u32(2);
        assert_eq!(check(wrong_owner), Err(UnsupportedMapping));
        let mut wrong_end = imported(0);
        wrong_end.original_end_pos.0 += 1;
        assert_eq!(check(wrong_end), Err(UnsupportedMapping));
        let mut original_overflow = imported(0);
        original_overflow.original_start_pos = BytePos(u32::MAX);
        assert_eq!(check(original_overflow), Err(UnsupportedMapping));
        let mut translated_overflow = imported(0);
        Arc::get_mut(&mut translated_overflow.translated_source_file).unwrap().start_pos =
            BytePos(u32::MAX);
        assert_eq!(check(translated_overflow), Err(UnsupportedMapping));
        let mut not_foreign = imported(0);
        Arc::get_mut(&mut not_foreign.translated_source_file).unwrap().external_src =
            FreezeLock::new(ExternalSource::Unneeded);
        assert_eq!(check(not_foreign), Err(UnsupportedMapping));
    }

    #[test]
    fn source_totals_reject_overflow_and_zero_budgets_accept_empty_coverage() {
        use ExternalSourceSnapshotError::*;
        let unlimited = ExternalSourceSnapshotLimits {
            metadata: limits(),
            max_files: usize::MAX,
            max_declared_source_bytes: usize::MAX,
        };
        let mut overflowing_files = usize::MAX;
        let mut overflowing_bytes = usize::MAX;
        assert_eq!(
            validate_source_inputs(
                1,
                1,
                &[Some(imported(0))],
                unlimited,
                &mut overflowing_files,
                &mut 0
            ),
            Err(FileBudget)
        );
        assert_eq!(
            validate_source_inputs(
                1,
                1,
                &[Some(imported(0))],
                unlimited,
                &mut 0,
                &mut overflowing_bytes
            ),
            Err(SourceByteBudget)
        );
        let zero = ExternalSourceSnapshotLimits {
            max_files: 0,
            max_declared_source_bytes: 0,
            ..unlimited
        };
        assert_eq!(validate_source_inputs(1, 0, &[], zero, &mut 0, &mut 0), Ok(()));
    }

    fn limits() -> MetadataSnapshotLimits {
        MetadataSnapshotLimits { max_crates: 2, max_metadata_bytes: 6, max_mapping_entries: 4 }
    }

    fn snapshot(bytes: &[u8], map: &[u32]) -> ExternalMetadataSnapshot {
        let mut b = SnapshotBuilder::new(limits());
        b.push(1, bytes, map.iter().copied()).unwrap();
        b.finish().unwrap()
    }

    #[test]
    fn owns_exact_bytes_and_compares_mapping() {
        let mut bytes = vec![1, 2, 3];
        let owned = snapshot(&bytes, &[1]);
        bytes[0] = 9;
        assert_ne!(owned, snapshot(&bytes, &[1]));
        assert_eq!(owned, snapshot(&[1, 2, 3], &[1]));
        assert_ne!(owned, snapshot(&[1, 2, 3], &[1, 1]));
    }

    #[test]
    fn budgets_include_all_crates_and_accept_exact_boundary() {
        let mut b = SnapshotBuilder::new(limits());
        b.push(1, &[1, 2, 3], [1, 2].into_iter()).unwrap();
        b.push(2, &[4, 5, 6], [2, 1].into_iter()).unwrap();
        assert_eq!(b.push(3, &[7], [3].into_iter()), Err(MetadataSnapshotError::CrateBudget));
        assert!(b.finish().is_ok());
        let mut b = SnapshotBuilder::new(limits());
        assert_eq!(b.push(1, &[0; 7], [1].into_iter()), Err(MetadataSnapshotError::MetadataBudget));
        assert_eq!(b.push(1, &[1], [1; 5].into_iter()), Err(MetadataSnapshotError::MappingBudget));
    }

    #[test]
    fn rejects_incomplete_or_ambiguous_numeric_mapping() {
        let mut b = SnapshotBuilder::new(limits());
        assert_eq!(b.push(1, &[1], [].into_iter()), Err(MetadataSnapshotError::UnsupportedMapping));
        assert_eq!(
            b.push(1, &[1], [2].into_iter()),
            Err(MetadataSnapshotError::UnsupportedMapping)
        );
        b.push(1, &[1], [1, 2].into_iter()).unwrap();
        assert_eq!(b.finish(), Err(MetadataSnapshotError::UnsupportedMapping));
        let mut b = SnapshotBuilder::new(limits());
        b.push(1, &[1], [1].into_iter()).unwrap();
        b.push(1, &[1], [1].into_iter()).unwrap();
        assert_eq!(b.finish(), Err(MetadataSnapshotError::UnsupportedMapping));
    }

    #[test]
    fn numeric_crate_assignment_and_mapping_order_are_inputs() {
        let make = |map: [u32; 3]| {
            let mut b = SnapshotBuilder::new(limits());
            b.push(1, &[1], map.into_iter()).unwrap();
            b.push(2, &[2], [2].into_iter()).unwrap();
            b.finish().unwrap()
        };
        assert_ne!(make([1, 1, 2]), make([1, 2, 1]));
        let mut b = SnapshotBuilder::new(limits());
        b.push(2, &[1], [2].into_iter()).unwrap();
        assert_ne!(snapshot(&[1], &[1]), b.finish().unwrap());
    }

    #[test]
    fn checked_totals_reject_overflow() {
        let mut b = SnapshotBuilder::new(MetadataSnapshotLimits {
            max_crates: usize::MAX,
            max_metadata_bytes: usize::MAX,
            max_mapping_entries: usize::MAX,
        });
        b.bytes = usize::MAX;
        assert_eq!(b.push(1, &[1], [1].into_iter()), Err(MetadataSnapshotError::MetadataBudget));
        b.bytes = 0;
        b.mappings = usize::MAX;
        assert_eq!(b.push(1, &[1], [1].into_iter()), Err(MetadataSnapshotError::MappingBudget));
    }

    #[test]
    fn empty_store_is_valid_with_zero_budgets() {
        let limits =
            MetadataSnapshotLimits { max_crates: 0, max_metadata_bytes: 0, max_mapping_entries: 0 };
        assert!(SnapshotBuilder::new(limits).finish().unwrap().crates.is_empty());
    }
}
