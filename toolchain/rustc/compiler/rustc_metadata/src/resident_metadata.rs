//! Exact, bounded external metadata inputs for resident-compiler identity work.
//!
//! Equality here is only an input prerequisite. It does not establish that external
//! DefIds, source positions, hygiene, or interned values can cross compiler contexts.
//! In particular this API must not be used to bypass resident reuse rejection.

use crate::creader::CStore;

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
