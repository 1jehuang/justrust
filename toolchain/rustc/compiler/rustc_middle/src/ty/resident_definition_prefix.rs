//! Nonfreezing, local-only observations of the currently allocated definition prefix.
//!
//! A prefix is NOT a complete crate identity, analysis result, or reuse authorization.
//! Equality says nothing about source, bodies, external crates, invocation options,
//! or definitions allocated after capture. Callers must separately establish those
//! facts. No query dependency is recorded. Never use this alone as a query cache key.

use rustc_hir::definitions::{DefKey, DefPathData, Definitions};
use rustc_span::def_id::LOCAL_CRATE;

use super::TyCtxt;

/// Limits for retained entries and bytes, including entry storage and path framing.
/// Allocator bookkeeping/capacity rounding and the existing native table are excluded.
#[derive(Clone, Copy, Debug)]
pub struct PrefixBudget {
    pub max_definitions: usize,
    pub max_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrefixError {
    DefinitionBudget,
    ByteBudget,
    /// IDs must be dense, unique, and in native allocation order, rooted at zero.
    InvalidNumericMapping,
    /// Only the root may lack a parent, and each parent must precede its child.
    InvalidParent,
    InvalidRoot,
}

#[derive(Debug, PartialEq, Eq)]
struct Definition {
    crate_index: u32,
    def_index: u32,
    // Exact, versioned, length-framed textual path components, not Symbol IDs.
    path: Vec<u8>,
}

/// An owned point-in-time prefix, deliberately distinct from any revision identity.
/// There is no public constructor from unvalidated numeric mappings.
#[derive(Debug)]
pub struct PrefixSnapshot {
    definitions: Vec<Definition>,
}

impl PrefixSnapshot {
    pub fn count(&self) -> usize {
        self.definitions.len()
    }

    /// Borrow the bounded observations in native allocation order without allocating.
    /// Each item is `(crate_index, def_index, canonical_path_bytes)`. The bytes use
    /// this module's versioned framing, not a display path or a complete identity.
    /// Iteration grants neither identity completeness nor query reuse authorization.
    pub fn entries(&self) -> impl ExactSizeIterator<Item = (u32, u32, &[u8])> + '_ {
        self.definitions
            .iter()
            .map(|entry| (entry.crate_index, entry.def_index, entry.path.as_slice()))
    }

    /// True only when every observed numeric ID and canonical path is unchanged.
    /// `later` may contain appended definitions. This does not prove completeness,
    /// temporal ordering, or that either snapshot came from the same compilation.
    pub fn is_preserved_by(&self, later: &Self) -> bool {
        later.definitions.starts_with(&self.definitions)
    }
}

/// Hold only the nonfreezing read guard while copying the current native table.
/// Releases the guard before returning, allowing allocation to continue. Does not
/// invoke analysis, `definitions()`, `iter_local_def_id()`, or any other query.
pub fn capture_definition_prefix(
    tcx: TyCtxt<'_>,
    budget: PrefixBudget,
) -> Result<PrefixSnapshot, PrefixError> {
    let definitions = tcx.definitions_untracked();
    capture_definitions(&definitions, budget)
}

fn capture_definitions(
    definitions: &Definitions,
    budget: PrefixBudget,
) -> Result<PrefixSnapshot, PrefixError> {
    capture_keys(
        definitions.num_definitions(),
        definitions.enumerated_keys_and_path_hashes().map(|(id, key, _)| (id.as_u32(), *key)),
        budget,
    )
}

fn capture_keys(
    count: usize,
    keys: impl Iterator<Item = (u32, DefKey)>,
    budget: PrefixBudget,
) -> Result<PrefixSnapshot, PrefixError> {
    if count == 0 {
        return Err(PrefixError::InvalidRoot);
    }
    if count > budget.max_definitions {
        return Err(PrefixError::DefinitionBudget);
    }
    let storage =
        count.checked_mul(std::mem::size_of::<Definition>()).ok_or(PrefixError::ByteBudget)?;
    let mut remaining = budget.max_bytes.checked_sub(storage).ok_or(PrefixError::ByteBudget)?;
    // Check the complete entry allocation before requesting it.
    let mut definitions: Vec<Definition> = Vec::with_capacity(count);
    for (id, key) in keys {
        let position = definitions.len();
        if position >= count || usize::try_from(id).ok() != Some(position) {
            return Err(PrefixError::InvalidNumericMapping);
        }
        let component = key.disambiguated_data;
        let (tag, name) = path_component(component.data);
        let parent_path = match (position, key.parent, component.data) {
            (0, None, DefPathData::CrateRoot) if component.disambiguator == 0 => {
                &b"resident-definition-prefix-path-v1"[..]
            }
            (0, _, _) | (_, _, DefPathData::CrateRoot) => return Err(PrefixError::InvalidRoot),
            (_, Some(parent), _) => {
                let parent =
                    usize::try_from(parent.as_u32()).map_err(|_| PrefixError::InvalidParent)?;
                if parent >= position {
                    return Err(PrefixError::InvalidParent);
                }
                definitions[parent].path.as_slice()
            }
            _ => return Err(PrefixError::InvalidParent),
        };
        // All fields are length-framed, including absent symbol text as empty.
        // Variant tags distinguish absent text from an explicitly empty name.
        let name = name.as_ref().map(|name| name.as_str()).unwrap_or("").as_bytes();
        let size = parent_path
            .len()
            .checked_add(8)
            .and_then(|n| n.checked_add(tag.len()))
            .and_then(|n| n.checked_add(8))
            .and_then(|n| n.checked_add(name.len()))
            .and_then(|n| n.checked_add(4))
            .ok_or(PrefixError::ByteBudget)?;
        remaining = remaining.checked_sub(size).ok_or(PrefixError::ByteBudget)?;
        // No native DefPath::make call: it allocates its ancestor vector before
        // a caller can bound it. Parent paths are already validated and owned.
        let mut path = Vec::with_capacity(size);
        path.extend_from_slice(parent_path);
        path.extend_from_slice(&(tag.len() as u64).to_le_bytes());
        path.extend_from_slice(tag.as_bytes());
        path.extend_from_slice(&(name.len() as u64).to_le_bytes());
        path.extend_from_slice(name);
        path.extend_from_slice(&component.disambiguator.to_le_bytes());
        definitions.push(Definition { crate_index: LOCAL_CRATE.as_u32(), def_index: id, path });
    }
    if definitions.len() != count {
        return Err(PrefixError::InvalidNumericMapping);
    }
    Ok(PrefixSnapshot { definitions })
}

// The completed-revision encoder is private to its module. Keep this exhaustive,
// explicit textual encoding independent rather than exporting identity machinery.
fn path_component(data: DefPathData) -> (&'static str, Option<rustc_span::Symbol>) {
    match data {
        DefPathData::CrateRoot => ("CrateRoot", None),
        DefPathData::Impl => ("Impl", None),
        DefPathData::ForeignMod => ("ForeignMod", None),
        DefPathData::Use => ("Use", None),
        DefPathData::GlobalAsm => ("GlobalAsm", None),
        DefPathData::TypeNs(name) => ("TypeNs", Some(name)),
        DefPathData::ValueNs(name) => ("ValueNs", Some(name)),
        DefPathData::MacroNs(name) => ("MacroNs", Some(name)),
        DefPathData::LifetimeNs(name) => ("LifetimeNs", Some(name)),
        DefPathData::Closure => ("Closure", None),
        DefPathData::TestBinderConstraints => ("TestBinderConstraints", None),
        DefPathData::Ctor => ("Ctor", None),
        DefPathData::AnonConst => ("AnonConst", None),
        DefPathData::OpaqueTy => ("OpaqueTy", None),
        DefPathData::OpaqueLifetime(name) => ("OpaqueLifetime", Some(name)),
        DefPathData::AnonAssocTy(name) => ("AnonAssocTy", Some(name)),
        DefPathData::SyntheticCoroutineBody => ("SyntheticCoroutineBody", None),
        DefPathData::NestedStatic => ("NestedStatic", None),
    }
}

#[cfg(test)]
mod tests {
    use rustc_data_structures::sync::FreezeLock;
    use rustc_hir::definitions::{DisambiguatedDefPathData, PerParentDisambiguatorState};
    use rustc_span::def_id::{CRATE_DEF_ID, DefIndex, StableCrateId};
    use rustc_span::{Symbol, create_session_globals_then, edition::Edition};

    use super::*;

    const BUDGET: PrefixBudget = PrefixBudget { max_definitions: 100, max_bytes: 65536 };

    #[test]
    fn native_capture_leaves_allocation_open_and_preserves_append() {
        create_session_globals_then(Edition::Edition2024, &[], None, || {
            let table = FreezeLock::new(Definitions::new(StableCrateId::new(
                Symbol::intern("prefix_test"),
                false,
                vec![],
                "test",
            )));
            let before = capture_definitions(&table.read(), BUDGET).unwrap();
            assert_eq!(before.count(), 1);
            assert!(!table.is_frozen());
            let mut disambiguators = PerParentDisambiguatorState::new(CRATE_DEF_ID);
            let id = table.write().create_def(
                CRATE_DEF_ID,
                DefPathData::TypeNs(Symbol::intern("Added")),
                &mut disambiguators,
            );
            assert_eq!(id.local_def_index.as_u32(), 1);
            let after = capture_definitions(&table.read(), BUDGET).unwrap();
            assert_eq!(after.count(), 2);
            assert!(before.is_preserved_by(&after));
            assert!(!after.is_preserved_by(&before));
            assert!(!table.is_frozen());
        });
    }

    fn key(parent: Option<u32>, data: DefPathData, disambiguator: u32) -> DefKey {
        DefKey {
            parent: parent.map(DefIndex::from_u32),
            disambiguated_data: DisambiguatedDefPathData { data, disambiguator },
        }
    }

    #[test]
    fn entries_borrow_exact_bounded_paths_in_native_order() {
        let root = key(None, DefPathData::CrateRoot, 0);
        let child = key(Some(0), DefPathData::Closure, 0);
        let snapshot = capture_keys(2, [(0, root), (1, child)].into_iter(), BUDGET).unwrap();
        let mut entries = snapshot.entries();
        assert_eq!(entries.len(), snapshot.count());
        for (index, definition) in snapshot.definitions.iter().enumerate() {
            let (crate_index, def_index, path) = entries.next().unwrap();
            assert_eq!(crate_index, LOCAL_CRATE.as_u32());
            assert_eq!(def_index as usize, index);
            assert_eq!(path, definition.path.as_slice());
            assert_eq!(path.as_ptr(), definition.path.as_ptr());
            assert_eq!(entries.len(), snapshot.count() - index - 1);
        }
        assert_eq!(entries.next(), None);
    }

    #[test]
    fn changed_prefix_and_malformed_numeric_mappings_are_rejected() {
        let root = key(None, DefPathData::CrateRoot, 0);
        let child = key(Some(0), DefPathData::Closure, 0);
        let capture = |items: Vec<_>| capture_keys(items.len(), items.into_iter(), BUDGET);
        let first = capture(vec![(0, root), (1, child)]).unwrap();
        let changed = capture(vec![(0, root), (1, key(Some(0), DefPathData::Ctor, 0))]).unwrap();
        assert!(!first.is_preserved_by(&changed));
        let changed = capture(vec![(0, root), (1, key(Some(0), DefPathData::Closure, 1))]).unwrap();
        assert!(!first.is_preserved_by(&changed));
        for items in [vec![(0, root), (0, child)], vec![(1, root)], vec![(0, root), (2, child)]] {
            assert_eq!(capture(items).unwrap_err(), PrefixError::InvalidNumericMapping);
        }
        assert_eq!(
            capture(vec![(0, root), (1, key(Some(1), DefPathData::Closure, 0))]).unwrap_err(),
            PrefixError::InvalidParent
        );
        assert_eq!(capture(vec![(0, child)]).unwrap_err(), PrefixError::InvalidRoot);
        assert_eq!(
            capture_keys(2, [(0, root)].into_iter(), BUDGET).unwrap_err(),
            PrefixError::InvalidNumericMapping
        );
    }

    #[test]
    fn textual_names_and_namespaces_are_exact() {
        create_session_globals_then(Edition::Edition2024, &[], None, || {
            let root = key(None, DefPathData::CrateRoot, 0);
            let capture = |data| {
                capture_keys(2, [(0, root), (1, key(Some(0), data, 0))].into_iter(), BUDGET)
                    .unwrap()
            };
            let first = capture(DefPathData::TypeNs(Symbol::intern("ab")));
            assert!(!first.is_preserved_by(&capture(DefPathData::TypeNs(Symbol::intern("a")))));
            assert!(!first.is_preserved_by(&capture(DefPathData::ValueNs(Symbol::intern("ab")))));
        });
    }

    #[test]
    fn bounds_are_checked_before_entry_or_path_allocation() {
        let root = key(None, DefPathData::CrateRoot, 0);
        let never_iterate = std::iter::from_fn(|| -> Option<(u32, DefKey)> {
            panic!("budget must reject before iteration")
        });
        assert_eq!(
            capture_keys(2, never_iterate, PrefixBudget { max_definitions: 1, ..BUDGET })
                .unwrap_err(),
            PrefixError::DefinitionBudget
        );
        assert_eq!(
            capture_keys(
                usize::MAX,
                std::iter::empty(),
                PrefixBudget { max_definitions: usize::MAX, max_bytes: usize::MAX }
            )
            .unwrap_err(),
            PrefixError::ByteBudget
        );
        assert_eq!(
            capture_keys(
                1,
                [(0, root)].into_iter(),
                PrefixBudget { max_bytes: std::mem::size_of::<Definition>(), ..BUDGET }
            )
            .unwrap_err(),
            PrefixError::ByteBudget
        );
    }
}
