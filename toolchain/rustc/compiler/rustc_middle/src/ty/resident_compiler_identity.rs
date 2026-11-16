//! Compiler-derived, owned identity prerequisites for local-only revisions.
//!
//! The completed builder finishes analysis and freezes allocation. The preanalysis
//! builder captures only the allocated prefix after resolver/HIR materialization.
//! That prefix is not completeness or standalone reuse authorization. Neither
//! builder retains a context, wires queries, or authorizes changed source. Capture
//! while the native source revision guard is still live.
//! External crates are deliberately unsupported, including implicit sysroot crates.
//! The caller must freeze *exact* semantic invocation material (including environment,
//! target/toolchain identity, version overrides, build/proc-macro inputs and options),
//! not command-line hashes. Neither this builder nor `RevisionIdentity` can establish
//! the completeness of caller-owned material.

use std::collections::BTreeSet;
use std::sync::Arc;

use rustc_span::def_id::LOCAL_CRATE;
use rustc_span::{FileName, RemapPathScopeComponents};
use rustc_structures::CrateType;

use super::TyCtxt;
use super::resident_definition_prefix::{PrefixBudget, PrefixError, capture_definition_prefix};
use super::resident_identity::{
    CrateIdentity, DefinitionIdentity, IdentityBudget, ResetReason, RevisionIdentity,
};

/// Borrowed immutable invocation material. Copies are made only within the budget.
/// These bytes must be canonical exact encodings, not fingerprints or Debug output.
pub struct FrozenSemanticInvocation<'a> {
    pub namespace: &'a [u8],
    pub options: &'a [u8],
    /// Non-source semantic inputs, including generated inputs not in the source map.
    pub body_revision: &'a [u8],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompilerIdentityError {
    Budget(ResetReason),
    UnsupportedExternalCrates,
    ActiveSourceViewUnavailable,
    /// An active file has no exact frozen native-loader raw input.
    UnfrozenSource,
    InvalidBodyOwner,
    InvalidDefinitionPrefix(PrefixError),
}

/// Capture real numeric IDs and canonical local definition paths in allocation order.
///
/// Initial safe subset: no external crates and only files covered by the native
/// loader's exact frozen raw snapshot. Input strings and generated/imported files
/// without frozen raw inputs are rejected, never reread or guessed from normalized
/// text. BOM and CRLF bytes are retained unchanged.
/// `active_source_files` is mandatory: disabled, loading and poisoned views all fail.
/// Source version indices and BytePos allocation are retained, so equivalent source
/// maps with different numeric assignments intentionally do not compare reusable.
/// Real filenames encode every public scoped name/working-directory/embeddable-name
/// projection plus the local name, without lossy Unicode conversion. Private unused
/// remapping representation is not claimed as an independently reconstructed identity.
/// Callers must include exact path remapping options in `options`.
pub fn build_revision_identity(
    tcx: TyCtxt<'_>,
    invocation: FrozenSemanticInvocation<'_>,
    budget: IdentityBudget,
) -> Result<RevisionIdentity, CompilerIdentityError> {
    // This completed path deliberately finishes analysis and freezes allocation.
    let _ = tcx.definitions();
    build_preanalysis_identity(tcx, invocation, budget)
}

/// Capture only the currently allocated definition prefix before analysis.
///
/// The driver must first materialize resolver/HIR crate inputs. This function
/// neither runs analysis nor freezes allocation. Appended definitions cannot alter
/// captured IDs, but this observation is NOT completeness or reuse authorization.
/// Reuse requires a completed old snapshot with exactly the same definition count,
/// every definition and every other identity field, plus the native domain gates.
/// Changed raw sources and body inputs must always reject reuse.
pub fn build_preanalysis_identity(
    tcx: TyCtxt<'_>,
    invocation: FrozenSemanticInvocation<'_>,
    budget: IdentityBudget,
) -> Result<RevisionIdentity, CompilerIdentityError> {
    use CompilerIdentityError as Error;
    if !tcx.crates(()).is_empty() {
        return Err(Error::UnsupportedExternalCrates);
    }
    if budget.max_crates == 0 {
        return Err(Error::Budget(ResetReason::CrateBudget));
    }
    let mut remaining = budget.max_identity_bytes;
    let namespace = copy_bytes(invocation.namespace, &mut remaining)?;
    let options = copy_bytes(invocation.options, &mut remaining)?;

    let mut krate = Bytes::new(&mut remaining);
    krate.bytes(b"resident-local-crate-v1")?;
    krate.bytes(tcx.crate_name(LOCAL_CRATE).as_str().as_bytes())?;
    krate.u64(tcx.stable_crate_id(LOCAL_CRATE).as_u64())?;
    krate.bytes(tcx.sess.cfg_version.as_bytes())?;
    krate.u32(u32::from(tcx.crate_types().contains(&CrateType::Executable)))?;
    krate.u64(tcx.sess.opts.cg.metadata.len() as u64)?;
    for metadata in &tcx.sess.opts.cg.metadata {
        krate.bytes(metadata.as_bytes())?;
    }
    // Options also contain the exact stable-ID version override, if any.
    let crates = vec![CrateIdentity { stable: krate.finish(), crate_index: LOCAL_CRATE.as_u32() }];

    let mut owners = BTreeSet::new();
    let mut bodies = Bytes::new(&mut remaining);
    bodies.bytes(b"resident-body-inputs-v1")?;
    bodies.bytes(invocation.body_revision)?;
    for owner in tcx.hir_body_owners() {
        if owners.len() >= budget.max_definitions {
            return Err(Error::Budget(ResetReason::DefinitionBudget));
        }
        let owner_index = owner.local_def_index.as_u32();
        if !owners.insert(owner_index) {
            return Err(Error::InvalidBodyOwner);
        }
        let body = tcx.hir_body_owned_by(owner).id().hir_id;
        bodies.u32(owner_index)?;
        bodies.u32(body.owner.def_id.local_def_index.as_u32())?;
        bodies.u32(body.local_id.as_u32())?;
    }
    let body_revision = bodies.finish();

    // Both builders consume the same bounded, nonfreezing canonical path encoding.
    let prefix = capture_definition_prefix(
        tcx,
        PrefixBudget { max_definitions: budget.max_definitions, max_bytes: remaining },
    )
    .map_err(|error| match error {
        PrefixError::DefinitionBudget => Error::Budget(ResetReason::DefinitionBudget),
        PrefixError::ByteBudget => Error::Budget(ResetReason::IdentityByteBudget),
        other => Error::InvalidDefinitionPrefix(other),
    })?;
    let mut definitions = Vec::with_capacity(prefix.count());
    for (crate_index, index, path) in prefix.entries() {
        definitions.push(definition_identity(
            crate_index,
            index,
            path,
            owners.remove(&index).then_some(index),
            &mut remaining,
        )?);
    }
    drop(prefix);
    if !owners.is_empty() {
        return Err(Error::InvalidBodyOwner);
    }

    let files = tcx
        .sess
        .source_map()
        .active_source_files()
        .map_err(|_| Error::ActiveSourceViewUnavailable)?
        .ok_or(Error::ActiveSourceViewUnavailable)?;
    // The source-map API bounds path + raw-text copies before allocating them.
    // Its snapshot is not a coverage guarantee: check every active Arc explicitly.
    let mut raw_files = tcx
        .sess
        .source_map()
        .resident_raw_source_files(remaining)
        .map_err(|_| Error::ActiveSourceViewUnavailable)?
        .ok_or(Error::ActiveSourceViewUnavailable)?;
    if raw_files.len() != files.len() {
        return Err(Error::UnfrozenSource);
    }
    let mut source = Bytes::new(&mut remaining);
    source.bytes(b"resident-active-raw-sources-v2")?;
    source.u64(files.len() as u64)?;
    for (version, file) in &files {
        if file.cnum != LOCAL_CRATE {
            return Err(Error::UnsupportedExternalCrates);
        }
        let raw_index = raw_files
            .iter()
            .position(|raw| Arc::ptr_eq(&raw.file, file))
            .ok_or(Error::UnfrozenSource)?;
        // Consume each matching entry exactly once. Equal names, hashes, contents
        // or BytePos values cannot substitute for the actual active SourceFile.
        let raw = raw_files.swap_remove(raw_index);
        source.u64(*version as u64)?;
        source.u32(file.start_pos.0)?;
        source.u32(file.normalized_source_len.0)?;
        source.u32(file.unnormalized_source_len)?;
        encode_file_name(&mut source, &file.name)?;
        source.bytes(raw.path.as_os_str().as_encoded_bytes())?;
        source.bytes(raw.raw.as_bytes())?;
    }
    if !raw_files.is_empty() {
        return Err(Error::UnfrozenSource);
    }
    // Detect loads/poisoning during capture. This remains only a point-in-time
    // prerequisite, not permission to use a snapshot across native guards.
    let current_files = tcx
        .sess
        .source_map()
        .active_source_files()
        .map_err(|_| Error::ActiveSourceViewUnavailable)?
        .ok_or(Error::ActiveSourceViewUnavailable)?;
    if files.len() != current_files.len()
        || !files.iter().zip(&current_files).all(
            |((index, file), (current_index, current_file))| {
                index == current_index && Arc::ptr_eq(file, current_file)
            },
        )
    {
        return Err(Error::UnfrozenSource);
    }
    let identity = RevisionIdentity {
        namespace,
        options,
        crates,
        definitions,
        source: source.finish(),
        body_revision,
    };
    identity.check_budget(budget).map_err(Error::Budget)?;
    Ok(identity)
}

fn definition_identity(
    crate_index: u32,
    def_index: u32,
    canonical_path: &[u8],
    owner_index: Option<u32>,
    remaining: &mut usize,
) -> Result<DefinitionIdentity, CompilerIdentityError> {
    // Preserve PrefixSnapshot's canonical bytes verbatim in both builder modes.
    Ok(DefinitionIdentity {
        stable: copy_bytes(canonical_path, remaining)?,
        crate_index,
        def_index,
        owner_index,
    })
}

fn encode_file_name(bytes: &mut Bytes<'_>, name: &FileName) -> Result<(), CompilerIdentityError> {
    match name {
        FileName::Real(name) => {
            bytes.u32(0)?;
            match name.local_path() {
                Some(path) => {
                    bytes.u32(1)?;
                    bytes.bytes(path.as_os_str().as_encoded_bytes())?;
                }
                None => bytes.u32(0)?,
            }
            for scope in [
                RemapPathScopeComponents::MACRO,
                RemapPathScopeComponents::DIAGNOSTICS,
                RemapPathScopeComponents::DEBUGINFO,
                RemapPathScopeComponents::COVERAGE,
                RemapPathScopeComponents::DOCUMENTATION,
            ] {
                bytes.bytes(name.path(scope).as_os_str().as_encoded_bytes())?;
                let (directory, embeddable) = name.embeddable_name(scope);
                bytes.bytes(directory.as_os_str().as_encoded_bytes())?;
                bytes.bytes(embeddable.as_os_str().as_encoded_bytes())?;
            }
        }
        FileName::CfgSpec(hash) => {
            bytes.u32(1)?;
            bytes.u64(hash.as_u64())?;
        }
        FileName::Anon(hash) => {
            bytes.u32(2)?;
            bytes.u64(hash.as_u64())?;
        }
        FileName::MacroExpansion(hash) => {
            bytes.u32(3)?;
            bytes.u64(hash.as_u64())?;
        }
        FileName::ProcMacroSourceCode(hash) => {
            bytes.u32(4)?;
            bytes.u64(hash.as_u64())?;
        }
        FileName::CliCrateAttr(hash) => {
            bytes.u32(5)?;
            bytes.u64(hash.as_u64())?;
        }
        FileName::Custom(name) => {
            bytes.u32(6)?;
            bytes.bytes(name.as_bytes())?;
        }
        FileName::DocTest(path, line) => {
            bytes.u32(7)?;
            bytes.bytes(path.as_os_str().as_encoded_bytes())?;
            bytes.u64(*line as i64 as u64)?;
        }
        FileName::InlineAsm(hash) => {
            bytes.u32(8)?;
            bytes.u64(hash.as_u64())?;
        }
    }
    Ok(())
}

fn copy_bytes(value: &[u8], remaining: &mut usize) -> Result<Vec<u8>, CompilerIdentityError> {
    *remaining = remaining
        .checked_sub(value.len())
        .ok_or(CompilerIdentityError::Budget(ResetReason::IdentityByteBudget))?;
    Ok(value.to_vec())
}

/// Length-delimited fixed-endian encoding with a shared aggregate byte budget.
/// Charge before extending any retained buffer, including length prefixes.
struct Bytes<'a> {
    data: Vec<u8>,
    remaining: &'a mut usize,
}

impl<'a> Bytes<'a> {
    fn new(remaining: &'a mut usize) -> Self {
        Self { data: Vec::new(), remaining }
    }

    fn raw(&mut self, value: &[u8]) -> Result<(), CompilerIdentityError> {
        *self.remaining = self
            .remaining
            .checked_sub(value.len())
            .ok_or(CompilerIdentityError::Budget(ResetReason::IdentityByteBudget))?;
        self.data.extend_from_slice(value);
        Ok(())
    }

    fn u32(&mut self, value: u32) -> Result<(), CompilerIdentityError> {
        self.raw(&value.to_le_bytes())
    }
    fn u64(&mut self, value: u64) -> Result<(), CompilerIdentityError> {
        self.raw(&value.to_le_bytes())
    }

    fn bytes(&mut self, value: &[u8]) -> Result<(), CompilerIdentityError> {
        // Reject the complete field before retaining even its prefix.
        if self.remaining.checked_sub(8).and_then(|n| n.checked_sub(value.len())).is_none() {
            return Err(CompilerIdentityError::Budget(ResetReason::IdentityByteBudget));
        }
        self.u64(value.len() as u64)?;
        self.raw(value)
    }

    fn finish(self) -> Vec<u8> {
        self.data
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn framing_distinguishes_concatenation_and_empty_fields() {
        let mut budget_a = 100;
        let mut a = Bytes::new(&mut budget_a);
        a.bytes(b"ab").unwrap();
        a.bytes(b"c").unwrap();
        let mut budget_b = 100;
        let mut b = Bytes::new(&mut budget_b);
        b.bytes(b"a").unwrap();
        b.bytes(b"bc").unwrap();
        assert_ne!(a.finish(), b.finish());
        let mut budget = 8;
        let mut empty = Bytes::new(&mut budget);
        empty.bytes(b"").unwrap();
        assert_eq!(empty.finish(), vec![0; 8]);
        assert_eq!(budget, 0);
    }

    #[test]
    fn canonical_prefix_encoding_is_preserved_in_shared_definition_path() {
        let path = b"resident-definition-prefix-path-v1\0CrateRoot\0Closure\0";
        let mut completed_budget = path.len();
        let mut preanalysis_budget = path.len();
        let completed = definition_identity(0, 1, path, Some(1), &mut completed_budget).unwrap();
        let preanalysis =
            definition_identity(0, 1, path, Some(1), &mut preanalysis_budget).unwrap();
        assert_eq!(completed, preanalysis);
        assert_eq!(completed.stable, path);
        assert_eq!(completed_budget, 0);
        assert_eq!(preanalysis_budget, 0);
        assert!(definition_identity(0, 1, path, Some(1), &mut (path.len() - 1)).is_err());
    }

    #[test]
    fn exact_reuse_ignores_no_identity_field_or_definition_count() {
        use super::super::resident_identity::Compatibility;

        let budget = IdentityBudget { max_crates: 4, max_definitions: 4, max_identity_bytes: 4096 };
        let old = RevisionIdentity {
            namespace: b"namespace".to_vec(),
            options: b"options".to_vec(),
            crates: vec![CrateIdentity { stable: b"crate".to_vec(), crate_index: 0 }],
            definitions: vec![DefinitionIdentity {
                stable: b"canonical-root".to_vec(),
                crate_index: 0,
                def_index: 0,
                owner_index: Some(0),
            }],
            source: b"exact raw source\r\n".to_vec(),
            body_revision: b"body owner and HIR numeric mapping".to_vec(),
        };
        assert_eq!(old.compatibility(&old, budget), Compatibility::Reusable);
        let mutations: &[fn(&mut RevisionIdentity)] = &[
            |id| id.namespace.push(0),
            |id| id.options.push(0),
            |id| id.crates[0].stable.push(0),
            |id| id.crates[0].crate_index = 1,
            |id| id.definitions[0].stable.push(0),
            |id| id.definitions[0].crate_index = 1,
            |id| id.definitions[0].def_index = 1,
            |id| id.definitions[0].owner_index = None,
            |id| id.source.push(0),
            |id| id.body_revision.push(0),
            |id| id.definitions.clear(),
            |id| {
                id.definitions.push(DefinitionIdentity {
                    stable: b"appended canonical child".to_vec(),
                    crate_index: 0,
                    def_index: 1,
                    owner_index: None,
                });
            },
        ];
        for (index, mutate) in mutations.iter().enumerate() {
            let mut next = old.clone();
            mutate(&mut next);
            assert_ne!(old.compatibility(&next, budget), Compatibility::Reusable, "field {index}");
            assert!(old.authorize_reuse(&next, budget).is_err(), "field {index}");
        }
    }

    #[test]
    fn virtual_file_name_variant_is_part_of_exact_identity() {
        fn name(value: &FileName) -> Vec<u8> {
            let mut budget = 100;
            let mut bytes = Bytes::new(&mut budget);
            encode_file_name(&mut bytes, value).unwrap();
            bytes.finish()
        }
        assert_ne!(name(&FileName::Custom("a".into())), name(&FileName::Custom("a\0".into())));
        assert_ne!(name(&FileName::Custom("a".into())), name(&FileName::DocTest("a".into(), 0)));
    }

    #[test]
    fn raw_source_framing_preserves_bom_and_crlf() {
        fn raw(text: &str) -> Vec<u8> {
            let mut budget = 100;
            let mut bytes = Bytes::new(&mut budget);
            bytes.bytes(b"/crate/source.rs").unwrap();
            bytes.bytes(text.as_bytes()).unwrap();
            bytes.finish()
        }
        assert_ne!(raw("fn f() {}\n"), raw("fn f() {}\r\n"));
        assert_ne!(raw("fn f() {}\n"), raw("\u{feff}fn f() {}\n"));
        assert!(raw("\u{feff}fn f() {}\r\n").ends_with("\u{feff}fn f() {}\r\n".as_bytes()));
    }

    #[test]
    fn aggregate_budget_charged_before_retention() {
        let mut budget = 10;
        let mut a = Bytes::new(&mut budget);
        a.bytes(b"ab").unwrap();
        assert_eq!(a.finish().len(), 10);
        let mut b = Bytes::new(&mut budget);
        assert_eq!(
            b.bytes(b""),
            Err(CompilerIdentityError::Budget(ResetReason::IdentityByteBudget))
        );
        assert!(b.finish().is_empty());
        let mut budget = 9;
        let mut c = Bytes::new(&mut budget);
        assert!(c.bytes(b"ab").is_err());
        assert!(c.finish().is_empty());
        assert_eq!(budget, 9);
    }
}
