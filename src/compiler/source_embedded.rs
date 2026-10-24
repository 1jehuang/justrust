//! Loader-issued embedded source provenance, never inferred from a virtual path.
//!
//! The payload is immutable original source and configured transport only. It
//! retains no compiler context, token session, graph handle or shared budget.
use super::*;
#[cfg(test)]
#[path = "source_embedded_tests.rs"]
mod tests;

#[derive(Clone, Debug)]
pub(super) struct EmbeddedUnit {
    identity: Arc<()>,
    path: PathBuf,
    original: Arc<str>,
    prepared: Arc<str>,
    origins: Arc<[OriginSegment]>,
    chain: Arc<[PathBuf]>,
    children: Arc<[ChildModule]>,
    module: Arc<[String]>,
    attributes: Arc<[PreparedAttribute]>,
    tests: Arc<[source_attrs::TestItem]>,
    no_std: bool,
    forbid_unsafe_code: bool,
    terminal: bool,
    _admitted: super::super::expansion_budget::RetainedBytes,
}

impl EmbeddedUnit {
    /// Only the source loader's embedded-reader branches call this factory.
    pub(super) fn issue(
        unit: &FileUnit,
        budget: &super::super::expansion_budget::ExpansionBudget,
    ) -> Result<Self, SourceError> {
        let bytes = unit_transport_bytes(unit)?;
        let admitted = admit(budget, bytes)?;
        Ok(Self {
            identity: Arc::new(()),
            path: unit.path.clone(),
            original: unit.contents.clone(),
            prepared: Arc::from(unit.text.as_str()),
            origins: Arc::from(unit.origins.as_slice()),
            chain: Arc::from(unit.chain.as_slice()),
            children: Arc::from(unit.children.as_slice()),
            module: Arc::from(unit.module.as_slice()),
            attributes: Arc::from(unit.attributes.as_slice()),
            tests: Arc::from(unit.tests.as_slice()),
            no_std: unit.no_std,
            forbid_unsafe_code: unit.forbid_unsafe_code,
            terminal: unit.terminal.is_some(),
            _admitted: admitted,
        })
    }

    pub(super) fn validate(&self, unit: &FileUnit) -> Result<(), SourceError> {
        if self.path != unit.path
            || !Arc::ptr_eq(&self.original, &unit.contents)
            || self.prepared.as_ref() != unit.text
            || self.origins.as_ref() != unit.origins
            || self.chain.as_ref() != unit.chain
            || self.children.as_ref() != unit.children
            || self.module.as_ref() != unit.module
            || self.attributes.as_ref() != unit.attributes
            || self.tests.as_ref() != unit.tests
            || self.no_std != unit.no_std
            || self.forbid_unsafe_code != unit.forbid_unsafe_code
            || self.terminal != unit.terminal.is_some()
        {
            return Err(Loader::error(
                SourceErrorKind::Contract,
                None,
                "embedded source issuer does not match its original file transport",
            ));
        }
        Ok(())
    }
}

/// Assembly-owned attachment. A copied path or independently loaded text cannot
/// manufacture this record, even if its bytes equal an embedded file.
#[derive(Clone, Debug)]
pub(super) struct EmbeddedFile {
    file: FileId,
    unit: EmbeddedUnit,
}

impl EmbeddedFile {
    pub(super) fn attach(file: FileId, unit: &FileUnit) -> Result<Option<Self>, SourceError> {
        let Some(issuer) = &unit.embedded else {
            return Ok(None);
        };
        issuer.validate(unit)?;
        Ok(Some(Self {
            file,
            unit: issuer.clone(),
        }))
    }

    pub(super) fn validate(&self, source: &PreparedSource) -> Result<FileId, SourceError> {
        let original = source
            .sources
            .files
            .get(self.file.0 as usize)
            .ok_or_else(|| {
                Loader::error(
                    SourceErrorKind::Contract,
                    None,
                    "embedded source file is missing",
                )
            })?;
        if original.lexical_path != self.unit.path
            || !Arc::ptr_eq(&original.contents, &self.unit.original)
        {
            return Err(Loader::error(
                SourceErrorKind::Contract,
                None,
                "embedded source attachment changed its original file",
            ));
        }
        Ok(self.file)
    }
}

/// Exact final transport admitted by the assembler. Parser-visible fields are
/// public for ordinary source tooling, so an embedded issuer is imported only
/// after this private immutable receipt replays the complete assembled stream.
#[derive(Clone, Debug)]
pub(super) struct AssemblyTransport {
    text: Arc<str>,
    origins: Arc<[OriginSegment]>,
    attributes: Arc<[PreparedAttribute]>,
    roots: Arc<[FileId]>,
    policies: Arc<[(FileId, Arc<str>, bool, bool)]>,
    files: Arc<[(PathBuf, Arc<str>)]>,
    issuers: Arc<[(FileId, Arc<()>)]>,
    debug_assertions: bool,
    overflow_checks: bool,
    _admitted: super::super::expansion_budget::RetainedBytes,
}
impl AssemblyTransport {
    pub(super) fn issue(source: &PreparedSource) -> Result<Self, SourceError> {
        // This is one assembly operation's local admission ledger, not a shared
        // compiler budget. Only its immutable retained-byte receipt escapes.
        let budget = super::super::expansion_budget::ExpansionBudget::default();
        let bytes = source.text.len() as u128
            + (source.origins.len() * std::mem::size_of::<OriginSegment>()) as u128
            + attribute_bytes(&source.attributes)
            + (source.crate_roots.len() * std::mem::size_of::<FileId>()) as u128
            + (source.crate_policies.len() * std::mem::size_of::<(FileId, Arc<str>, bool, bool)>())
                as u128
            + source
                .sources
                .files
                .iter()
                .map(|file| {
                    (std::mem::size_of::<(PathBuf, Arc<str>)>()
                        + file.lexical_path.as_os_str().len()) as u128
                })
                .sum::<u128>()
            + (source.embedded_files.len() * std::mem::size_of::<(FileId, Arc<()>)>()) as u128;
        let admitted = admit(&budget, bounded_bytes(bytes)?)?;
        Ok(Self {
            text: Arc::from(source.text.as_str()),
            origins: Arc::from(source.origins.as_slice()),
            attributes: Arc::from(source.attributes.as_slice()),
            roots: Arc::from(source.crate_roots.as_slice()),
            policies: source
                .crate_policies
                .iter()
                .map(|policy| {
                    (
                        policy.root,
                        policy.original.clone(),
                        policy.no_std,
                        policy.forbid_unsafe_code,
                    )
                })
                .collect(),
            files: source
                .sources
                .files
                .iter()
                .map(|file| (file.lexical_path.clone(), file.contents.clone()))
                .collect(),
            issuers: source
                .embedded_files
                .iter()
                .map(|file| (file.file, file.unit.identity.clone()))
                .collect(),
            debug_assertions: source.debug_assertions,
            overflow_checks: source.overflow_checks,
            _admitted: admitted,
        })
    }
    pub(super) fn validate(&self, source: &PreparedSource) -> Result<(), SourceError> {
        let policies_match = self.policies.len() == source.crate_policies.len()
            && self.policies.iter().zip(&source.crate_policies).all(
                |((root, original, no_std, forbid_unsafe_code), policy)| {
                    *root == policy.root
                        && Arc::ptr_eq(original, &policy.original)
                        && *no_std == policy.no_std
                        && *forbid_unsafe_code == policy.forbid_unsafe_code
                },
            );
        let files_match = self.files.len() == source.sources.files.len()
            && self
                .files
                .iter()
                .zip(&source.sources.files)
                .all(|((path, original), file)| {
                    path == &file.lexical_path && Arc::ptr_eq(original, &file.contents)
                });
        let issuers_match = self.issuers.len() == source.embedded_files.len()
            && self
                .issuers
                .iter()
                .zip(&source.embedded_files)
                .all(|((id, issuer), file)| {
                    *id == file.file && Arc::ptr_eq(issuer, &file.unit.identity)
                });
        if self.text.as_ref() != source.text
            || self.origins.as_ref() != source.origins
            || self.attributes.as_ref() != source.attributes
            || self.roots.as_ref() != source.crate_roots
            || self.debug_assertions != source.debug_assertions
            || self.overflow_checks != source.overflow_checks
            || !policies_match
            || !files_match
            || !issuers_match
        {
            return Err(Loader::error(
                SourceErrorKind::Contract,
                None,
                "embedded source assembly changed after authenticated transport",
            ));
        }
        Ok(())
    }
}

fn bounded_bytes(bytes: u128) -> Result<u64, SourceError> {
    u64::try_from(bytes).map_err(|_| {
        Loader::error(
            SourceErrorKind::Limit,
            None,
            "embedded source receipt size overflow",
        )
    })
}
fn admit(
    budget: &super::super::expansion_budget::ExpansionBudget,
    bytes: u64,
) -> Result<super::super::expansion_budget::RetainedBytes, SourceError> {
    use super::super::expansion_budget::{Counter, Storage};
    budget
        .charge(Counter::Work, bytes)
        .map_err(|error| SourceError::from_token(error.into()))?;
    budget
        .reserve(Storage::Retained, bytes)
        .and_then(|lease| lease.retain())
        .map_err(|error| SourceError::from_token(error.into()))
}
fn attribute_bytes(attributes: &[PreparedAttribute]) -> u128 {
    attributes
        .iter()
        .map(|attribute| (std::mem::size_of::<PreparedAttribute>() + attribute.name.len()) as u128)
        .sum()
}
fn unit_transport_bytes(unit: &FileUnit) -> Result<u64, SourceError> {
    let paths = unit
        .chain
        .iter()
        .map(|path| (std::mem::size_of::<PathBuf>() + path.as_os_str().len()) as u128)
        .sum::<u128>();
    let children = unit
        .children
        .iter()
        .map(|child| {
            std::mem::size_of::<ChildModule>() as u128
                + child.path.as_os_str().len() as u128
                + child.directory.as_os_str().len() as u128
                + child
                    .module
                    .iter()
                    .map(|name| (std::mem::size_of::<String>() + name.len()) as u128)
                    .sum::<u128>()
        })
        .sum::<u128>();
    let tests = unit
        .tests
        .iter()
        .map(|test| {
            std::mem::size_of::<source_attrs::TestItem>() as u128
                + test
                    .path
                    .iter()
                    .map(|name| (std::mem::size_of::<String>() + name.len()) as u128)
                    .sum::<u128>()
                + test
                    .should_panic
                    .as_ref()
                    .and_then(|value| value.as_ref())
                    .map_or(0, |value| value.len() as u128)
        })
        .sum::<u128>();
    bounded_bytes(
        std::mem::size_of::<EmbeddedUnit>() as u128
            + unit.path.as_os_str().len() as u128
            + unit.text.len() as u128
            + (unit.origins.len() * std::mem::size_of::<OriginSegment>()) as u128
            + paths
            + children
            + tests
            + attribute_bytes(&unit.attributes)
            + unit
                .module
                .iter()
                .map(|name| (std::mem::size_of::<String>() + name.len()) as u128)
                .sum::<u128>(),
    )
}

/// Issued only when validated assembly records enter this source snapshot.
#[derive(Clone, Debug)]
pub(in crate::compiler) struct EmbeddedOriginalFile {
    file: crate::compiler::token_session::SourceFileHandle,
    issuer: Arc<()>,
    original: Arc<str>,
}
impl EmbeddedOriginalFile {
    pub(super) fn import(
        attachment: &EmbeddedFile,
        source: &PreparedSource,
        snapshot: &crate::compiler::token_session::SourceSnapshot,
    ) -> Result<Self, SourceError> {
        let id = attachment.validate(source)?;
        let file = snapshot
            .file(id.0 as usize)
            .map_err(SourceError::from_token)?;
        if snapshot.text(file).map_err(SourceError::from_token)?
            != attachment.unit.original.as_ref()
        {
            return Err(Loader::error(
                SourceErrorKind::Contract,
                None,
                "embedded source snapshot changed original bytes",
            ));
        }
        Ok(Self {
            file,
            issuer: attachment.unit.identity.clone(),
            original: attachment.unit.original.clone(),
        })
    }
    pub(in crate::compiler) fn authorizes(
        &self,
        snapshot: &crate::compiler::token_session::SourceSnapshot,
        span: crate::compiler::token_session::OriginalSpan,
    ) -> bool {
        let Ok(raw) = snapshot.render_span(span) else {
            return false;
        };
        snapshot.span(self.file, raw.lo, raw.hi).ok() == Some(span)
            && snapshot.text(self.file).ok() == Some(self.original.as_ref())
    }
    pub(in crate::compiler) fn same_issuer(&self, other: &Self) -> bool {
        self.file == other.file && Arc::ptr_eq(&self.issuer, &other.issuer)
    }
}
