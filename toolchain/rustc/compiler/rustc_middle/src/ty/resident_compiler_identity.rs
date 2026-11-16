//! Compiler-derived, owned identity prerequisite for a completed local-only revision.
//!
//! This does not retain a context, wire queries, authorize changed source, or prove
//! that analysis is complete. Call only after the native executor has finished all
//! source loading and semantic work, while its source revision guard is still live.
//! External crates are deliberately unsupported, including implicit sysroot crates.
//! The caller must freeze *exact* semantic invocation material (including environment,
//! target/toolchain identity, version overrides, build/proc-macro inputs and options),
//! not command-line hashes. Neither this builder nor `RevisionIdentity` can establish
//! the completeness of caller-owned material.

use std::collections::BTreeSet;

use rustc_hir::definitions::DefPathData;
use rustc_session::config::CrateType;
use rustc_span::def_id::LOCAL_CRATE;
use rustc_span::{FileName, RemapPathScopeComponents};

use super::TyCtxt;
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
    /// SourceFile exposes normalized text, not the original BOM/CRLF bytes.
    UnsupportedNormalizedSource,
    MissingSourceText,
    InvalidBodyOwner,
}

/// Capture real numeric IDs and canonical local definition paths in allocation order.
///
/// Initial safe subset: no external crates and no source normalization. Normalized
/// sources are rejected rather than reread from disk or guessed from normalized text.
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
    use CompilerIdentityError as Error;

    // Complete analysis before observing the crate universe. iter_local_def_id also
    // depends on analysis and freezes definition allocation when exhausted.
    let local_definitions = tcx.definitions();
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

    let mut definitions = Vec::new();
    for id in tcx.iter_local_def_id() {
        if definitions.len() >= budget.max_definitions {
            return Err(Error::Budget(ResetReason::DefinitionBudget));
        }
        let index = id.local_def_index.as_u32();
        let path = local_definitions.def_path(id);
        let mut stable = Bytes::new(&mut remaining);
        stable.bytes(b"resident-def-path-v1")?;
        stable.u64(path.data.len() as u64)?;
        for component in path.data {
            encode_path_data(&mut stable, component.data)?;
            stable.u32(component.disambiguator)?;
        }
        definitions.push(DefinitionIdentity {
            stable: stable.finish(),
            crate_index: LOCAL_CRATE.as_u32(),
            def_index: index,
            owner_index: owners.remove(&index).then_some(index),
        });
    }
    if !owners.is_empty() {
        return Err(Error::InvalidBodyOwner);
    }

    let files = tcx
        .sess
        .source_map()
        .active_source_files()
        .map_err(|_| Error::ActiveSourceViewUnavailable)?
        .ok_or(Error::ActiveSourceViewUnavailable)?;
    let mut source = Bytes::new(&mut remaining);
    source.bytes(b"resident-active-sources-v1")?;
    source.u64(files.len() as u64)?;
    for (version, file) in files {
        if file.cnum != LOCAL_CRATE {
            return Err(Error::UnsupportedExternalCrates);
        }
        let text = file.src.as_ref().ok_or(Error::MissingSourceText)?;
        if !file.normalized_pos.is_empty() || text.len() != file.unnormalized_source_len as usize {
            return Err(Error::UnsupportedNormalizedSource);
        }
        source.u64(version as u64)?;
        source.u32(file.start_pos.0)?;
        source.u32(file.normalized_source_len.0)?;
        encode_file_name(&mut source, &file.name)?;
        source.bytes(text.as_bytes())?;
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

fn encode_path_data(bytes: &mut Bytes<'_>, data: DefPathData) -> Result<(), CompilerIdentityError> {
    // Exhaustive tags, never Debug(Symbol), numeric Symbol IDs or def-path hashes.
    let (tag, symbol) = match data {
        DefPathData::CrateRoot => (0, None),
        DefPathData::Impl => (1, None),
        DefPathData::ForeignMod => (2, None),
        DefPathData::Use => (3, None),
        DefPathData::GlobalAsm => (4, None),
        DefPathData::TypeNs(name) => (5, Some(name)),
        DefPathData::ValueNs(name) => (6, Some(name)),
        DefPathData::MacroNs(name) => (7, Some(name)),
        DefPathData::LifetimeNs(name) => (8, Some(name)),
        DefPathData::Closure => (9, None),
        DefPathData::TestBinderConstraints => (10, None),
        DefPathData::Ctor => (11, None),
        DefPathData::AnonConst => (12, None),
        DefPathData::OpaqueTy => (13, None),
        DefPathData::OpaqueLifetime(name) => (14, Some(name)),
        DefPathData::AnonAssocTy(name) => (15, Some(name)),
        DefPathData::SyntheticCoroutineBody => (16, None),
        DefPathData::NestedStatic => (17, None),
    };
    bytes.u32(tag)?;
    if let Some(symbol) = symbol {
        bytes.bytes(symbol.as_str().as_bytes())?;
    }
    Ok(())
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
    fn anonymous_path_tags_and_disambiguators_are_not_collapsed() {
        fn component(data: DefPathData, disambiguator: u32) -> Vec<u8> {
            let mut budget = 100;
            let mut bytes = Bytes::new(&mut budget);
            encode_path_data(&mut bytes, data).unwrap();
            bytes.u32(disambiguator).unwrap();
            bytes.finish()
        }
        assert_ne!(component(DefPathData::Closure, 0), component(DefPathData::Ctor, 0));
        assert_ne!(component(DefPathData::Closure, 0), component(DefPathData::Closure, 1));
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
