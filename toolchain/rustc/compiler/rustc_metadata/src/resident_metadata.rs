//! Exact, bounded external metadata inputs for resident-compiler identity work.
//!
//! Equality here is only an input prerequisite. It does not establish that external
//! DefIds, source positions, hygiene, or interned values can cross compiler contexts.
//! In particular this API must not be used to bypass resident reuse rejection.

use crate::creader::CStore;
use rustc_span::{ExternalSource, Pos, SourceFile};
use std::sync::Arc;

/// The actual per-metadata decoder import relation. Shared here so capture can
/// inspect it without exposing the private decoder module or forcing an import.
#[derive(Clone)]
pub(crate) struct ImportedSourceFile {
    pub(crate) original_start_pos: rustc_span::BytePos,
    pub(crate) original_end_pos: rustc_span::BytePos,
    pub(crate) translated_source_file: Arc<SourceFile>,
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
