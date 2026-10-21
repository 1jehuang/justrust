//! Sealed source ownership and issuing authority for token-only handles.
//! No raw FileId/SourceSpan is accepted as an owned source reference.
use std::mem::size_of;
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::expansion_budget::{BudgetFailure, ByteLease, Counter, Depth, ExpansionBudget, Storage};
use super::expansion_span::{ExpansionRecord, SpanData, SpanOrigin, SyntheticReason, TraceFrame};
use super::hygiene::{ContextData, Transparency};
use super::source::source_map::{FileId, SourceFile, SourceMap, SourceSpan};
use super::source::{MAX_BYTES, MAX_FILES};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceLimitResource {
    Files,
    Bytes,
}

#[derive(Debug)]
pub enum TokenError {
    Budget(BudgetFailure),
    ForeignHandle,
    InvalidHandle,
    InvalidRange,
    InvalidUtf8Boundary,
    InvalidInput(&'static str),
    UnbalancedDelimiter {
        span: Option<SpanId>,
    },
    IdExhausted,
    /// Two distinct spellings share one atom fingerprint.
    AtomCollision,
    TraceTruncated,
    SourceLimit {
        resource: SourceLimitResource,
        used: usize,
        requested: usize,
        limit: usize,
    },
}

impl From<BudgetFailure> for TokenError {
    fn from(error: BudgetFailure) -> Self {
        Self::Budget(error)
    }
}

impl std::fmt::Display for TokenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Budget(error) => write!(f, "token budget: {error:?}"),
            Self::ForeignHandle => f.write_str("foreign token/source handle"),
            Self::InvalidHandle => f.write_str("invalid token/source handle"),
            Self::InvalidRange => f.write_str("invalid source/tree range"),
            Self::InvalidUtf8Boundary => f.write_str("source span is not on UTF-8 boundaries"),
            Self::InvalidInput(message) => f.write_str(message),
            Self::UnbalancedDelimiter { .. } => f.write_str("unbalanced token delimiter"),
            Self::IdExhausted => f.write_str("token identity space exhausted"),
            Self::AtomCollision => f.write_str("identifier atom fingerprint collision"),
            Self::TraceTruncated => f.write_str("bounded token provenance traversal truncated"),
            Self::SourceLimit {
                resource,
                used,
                requested,
                limit,
            } => write!(
                f,
                "source snapshot import limit {resource:?}: used {used}, requested {requested}, limit {limit}"
            ),
        }
    }
}
impl std::error::Error for TokenError {}

pub(super) fn allocate_identity(counter: &AtomicU64) -> Result<NonZeroU64, TokenError> {
    let mut current = counter.load(Ordering::Relaxed);
    loop {
        let successor = current.checked_add(1).ok_or(TokenError::IdExhausted)?;
        let issued = NonZeroU64::new(current).ok_or(TokenError::IdExhausted)?;
        match counter.compare_exchange_weak(
            current,
            successor,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => return Ok(issued),
            Err(observed) => current = observed,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct SourceSnapshotId(NonZeroU64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct SessionId(NonZeroU64);
/// Reserved owner of the crate-root syntax context. Never issued to a session.
const ROOT_CONTEXT_OWNER: SessionId = SessionId(NonZeroU64::MAX);

/// Content identity of an interned spelling: a fixed-key 128-bit fingerprint,
/// equal across sessions for equal text (PARSE-SPLIT.md step 2a). A session
/// rejects two distinct spellings with one fingerprint (`AtomCollision`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct AtomFingerprint([u64; 2]);
impl AtomFingerprint {
    pub fn of(text: &str) -> Self {
        // Fixed-basis 128-bit FNV-1a: deterministic across processes and
        // sessions. Collisions are rejected at interning, never aliased.
        const PRIME: u128 = 0x0000_0000_0100_0000_0000_0000_0000_013B;
        let mut state: u128 = 0x6c62_272e_07bb_0142_62b8_2175_6295_c58d;
        for byte in text.bytes() {
            state ^= u128::from(byte);
            state = state.wrapping_mul(PRIME);
        }
        // Two u64 lanes keep 8-byte alignment, so NameData/NameKey layouts do not grow.
        Self([(state >> 64) as u64, state as u64])
    }
    fn table_hash(self) -> u64 {
        self.0[0] ^ self.0[1]
    }
}
impl SessionId {
    #[cfg(test)]
    pub(super) fn bits(self) -> NonZeroU64 {
        self.0
    }
}

/// Explicit, crate-scoped set of owners whose handles a store admits
/// (PARSE-SPLIT.md blocker 1). Membership is never "any": the set is built
/// only from owners the store itself was issued, and is immutable once built.
/// Today every set holds exactly its own owner, so admission is unchanged.
/// Per-file sessions will extend it with the crate's sibling file owners.
/// Held inline (no heap) so the existing retained-byte accounting is exact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct CrateOwners<T: Copy + Eq> {
    issuer: T,
}
impl<T: Copy + Eq> CrateOwners<T> {
    /// The set containing exactly the issuing owner.
    pub(super) fn single(owner: T) -> Self {
        Self { issuer: owner }
    }
    pub(super) fn admits(&self, owner: T) -> bool {
        owner == self.issuer
    }
}

macro_rules! session_handle {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name { session: SessionId, index: u32 }
    )+};
}
session_handle!(SpanId, ExpansionId, SyntaxContextId, SymbolId, CrateId);
impl SyntaxContextId {
    /// The fixed crate-root context (no session owns it).
    pub const ROOT: Self = Self {
        session: ROOT_CONTEXT_OWNER,
        index: 0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SourceFileHandle {
    source: SourceSnapshotId,
    index: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct OriginalSpan {
    file: SourceFileHandle,
    lo: u32,
    hi: u32,
}

struct SealedSources {
    id: SourceSnapshotId,
    storage: SourceStorage,
    crate_roots: Vec<super::source::FileId>,
    // Drop after the backing. Clones retain this through the shared Arc, rather
    // than tying source ownership to any particular TokenSession lifetime.
    _lease: Option<ByteLease>,
}

enum SourceStorage {
    Map(SourceMap),
    Raw {
        path: PathBuf,
        contents: String,
        line_starts: Vec<u32>,
    },
}

struct SourceView<'a> {
    path: &'a Path,
    contents: &'a str,
    line_starts: &'a [u32],
}

impl SourceStorage {
    fn view(&self, index: usize) -> Result<SourceView<'_>, TokenError> {
        match self {
            Self::Map(map) => {
                let file = map.files.get(index).ok_or(TokenError::InvalidHandle)?;
                Ok(SourceView {
                    path: &file.lexical_path,
                    contents: &file.contents,
                    line_starts: &file.line_starts,
                })
            }
            Self::Raw {
                path,
                contents,
                line_starts,
            } if index == 0 => Ok(SourceView {
                path,
                contents,
                line_starts,
            }),
            Self::Raw { .. } => Err(TokenError::InvalidHandle),
        }
    }
}

/// Takes ownership of a bounded source map without copying its bytes.
/// Import preflights canonical source file/content limits before scanning contents.
/// It performs no I/O, configuration processing or semantic publication.
#[derive(Clone)]
pub struct SourceSnapshot {
    inner: Arc<SealedSources>,
}

impl SourceSnapshot {
    pub fn new(map: SourceMap) -> Result<Self, TokenError> {
        Self::validate_map(&map, None)?;
        Self::seal(SourceStorage::Map(map), None)
    }

    /// Copies only map metadata. Original contents remain shared with their
    /// source owner, whose source-byte accounting is not replaced by this lease.
    pub fn copy_borrowed(map: &SourceMap, budget: &ExpansionBudget) -> Result<Self, TokenError> {
        Self::copy_with_crates(map, &[], budget)
    }

    pub(super) fn copy_with_crates(
        map: &SourceMap,
        roots: &[super::source::FileId],
        budget: &ExpansionBudget,
    ) -> Result<Self, TokenError> {
        budget.check()?;
        Self::validate_map(map, Some(budget))?;
        let mut lease = Self::metadata_lease(budget)?;
        if !roots.is_empty() && roots.len() != map.files.len() {
            return Err(TokenError::InvalidInput(
                "incomplete source crate ownership",
            ));
        }
        let mut crate_roots = Self::copy_capacity(roots.len(), budget, &mut lease)?;
        for (index, &root) in roots.iter().enumerate() {
            budget.charge(Counter::Work, 1)?;
            if root.0 as usize > index || roots.get(root.0 as usize) != Some(&root) {
                return Err(TokenError::InvalidInput("invalid source crate root"));
            }
            crate_roots.push(root);
        }
        let mut files = Self::copy_capacity::<SourceFile>(map.files.len(), budget, &mut lease)?;
        for file in &map.files {
            let lexical_path = Self::copy_path(&file.lexical_path, budget, &mut lease)?;
            let mut line_starts =
                Self::copy_capacity::<u32>(file.line_starts.len(), budget, &mut lease)?;
            budget.charge(Counter::Work, file.line_starts.len() as u64)?;
            line_starts.extend_from_slice(&file.line_starts);
            files.push(SourceFile {
                lexical_path,
                contents: Arc::clone(&file.contents),
                line_starts,
            });
        }
        Self::seal_with_crates(
            SourceStorage::Map(SourceMap { files }),
            crate_roots,
            Some(lease),
        )
    }

    /// Creates one syntax-only identity source without preparation, cfg or I/O.
    /// Unlike borrowed map contents, the new text payload belongs to this lease.
    pub fn from_text(
        name: &Path,
        text: &str,
        budget: &ExpansionBudget,
    ) -> Result<Self, TokenError> {
        budget.check()?;
        if text.len() > MAX_BYTES {
            return Err(TokenError::SourceLimit {
                resource: SourceLimitResource::Bytes,
                used: 0,
                requested: text.len(),
                limit: MAX_BYTES,
            });
        }
        let mut lease = Self::metadata_lease(budget)?;
        let path = Self::copy_path(name, budget, &mut lease)?;
        budget.charge(Counter::Work, text.len() as u64)?;
        let mut contents = String::new();
        let base = lease.bytes();
        Self::reserve_payload(&mut lease, base, text.len(), 1)?;
        contents
            .try_reserve_exact(text.len())
            .map_err(|_| budget.poison_allocation())?;
        Self::reserve_payload(&mut lease, base, contents.capacity(), 1)?;
        contents.push_str(text);
        budget.charge(Counter::Work, text.len() as u64 + 1)?;
        let lines = text.bytes().filter(|byte| *byte == b'\n').count() + 1;
        let mut line_starts = Self::copy_capacity::<u32>(lines, budget, &mut lease)?;
        budget.charge(Counter::Work, text.len() as u64 + 1)?;
        line_starts.push(0);
        for (index, byte) in text.bytes().enumerate() {
            if byte == b'\n' {
                line_starts.push((index + 1) as u32);
            }
        }
        Self::seal(
            SourceStorage::Raw {
                path,
                contents,
                line_starts,
            },
            Some(lease),
        )
    }

    /// Exact sealed-owner equality, not equality of paths or source contents.
    pub fn same_snapshot(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }

    fn metadata_lease(budget: &ExpansionBudget) -> Result<ByteLease, TokenError> {
        Ok(budget.reserve(
            Storage::Retained,
            (size_of::<SealedSources>() + 2 * size_of::<usize>()) as u64,
        )?)
    }

    fn reserve_payload(
        lease: &mut ByteLease,
        base: u64,
        count: usize,
        size: usize,
    ) -> Result<(), TokenError> {
        let bytes = (count as u64)
            .checked_mul(size as u64)
            .and_then(|bytes| base.checked_add(bytes))
            .ok_or(TokenError::IdExhausted)?;
        Ok(lease.resize(bytes)?)
    }

    // On reconciliation failure the local backing drops before the caller's
    // lease. No allocation survives a failed constructor admission.
    fn copy_capacity<T>(
        count: usize,
        budget: &ExpansionBudget,
        lease: &mut ByteLease,
    ) -> Result<Vec<T>, TokenError> {
        let base = lease.bytes();
        Self::reserve_payload(lease, base, count, size_of::<T>())?;
        let mut values = Vec::new();
        values
            .try_reserve_exact(count)
            .map_err(|_| budget.poison_allocation())?;
        Self::reserve_payload(lease, base, values.capacity(), size_of::<T>())?;
        Ok(values)
    }

    fn copy_path(
        path: &Path,
        budget: &ExpansionBudget,
        lease: &mut ByteLease,
    ) -> Result<PathBuf, TokenError> {
        budget.charge(Counter::Work, path.as_os_str().len() as u64)?;
        let base = lease.bytes();
        Self::reserve_payload(lease, base, path.as_os_str().len(), 1)?;
        let mut copied = std::ffi::OsString::new();
        copied
            .try_reserve_exact(path.as_os_str().len())
            .map_err(|_| budget.poison_allocation())?;
        Self::reserve_payload(lease, base, copied.capacity(), 1)?;
        copied.push(path.as_os_str());
        Ok(PathBuf::from(copied))
    }

    fn validate_map(map: &SourceMap, budget: Option<&ExpansionBudget>) -> Result<(), TokenError> {
        if map.files.len() > MAX_FILES {
            return Err(TokenError::SourceLimit {
                resource: SourceLimitResource::Files,
                used: 0,
                requested: map.files.len(),
                limit: MAX_FILES,
            });
        }
        let mut bytes = 0usize;
        for file in &map.files {
            let requested = file.contents.len();
            bytes = bytes
                .checked_add(requested)
                .filter(|total| *total <= MAX_BYTES)
                .ok_or(TokenError::SourceLimit {
                    resource: SourceLimitResource::Bytes,
                    used: bytes,
                    requested,
                    limit: MAX_BYTES,
                })?;
        }
        u32::try_from(map.files.len()).map_err(|_| TokenError::InvalidRange)?;
        if let Some(budget) = budget {
            budget.charge(Counter::Work, map.files.len() as u64)?;
        }
        for file in &map.files {
            u32::try_from(file.contents.len()).map_err(|_| TokenError::InvalidRange)?;
            if let Some(budget) = budget {
                // Metadata may be arbitrarily large even when source bytes fit
                // their canonical cap. Admit traversal before examining it.
                budget.charge(Counter::Work, file.contents.len() as u64 + 1)?;
                budget.charge(Counter::Work, file.line_starts.len() as u64)?;
            }
            let expected = std::iter::once(0).chain(
                file.contents
                    .bytes()
                    .enumerate()
                    .filter_map(|(i, byte)| (byte == b'\n').then_some((i + 1) as u32)),
            );
            if !file.line_starts.iter().copied().eq(expected) {
                return Err(TokenError::InvalidInput(
                    "invalid original source line table",
                ));
            }
        }
        Ok(())
    }

    fn seal(storage: SourceStorage, lease: Option<ByteLease>) -> Result<Self, TokenError> {
        Self::seal_with_crates(storage, Vec::new(), lease)
    }

    fn seal_with_crates(
        storage: SourceStorage,
        crate_roots: Vec<super::source::FileId>,
        lease: Option<ByteLease>,
    ) -> Result<Self, TokenError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = match allocate_identity(&NEXT) {
            Ok(id) => SourceSnapshotId(id),
            Err(error) => {
                drop(storage);
                drop(lease);
                return Err(error);
            }
        };
        Ok(Self {
            // Stable std has no fallible Arc control-block constructor. The
            // small fixed metadata allocation is accounted by budgeted callers;
            // all variable-sized copied payload allocations are fallible.
            inner: Arc::new(SealedSources {
                id,
                storage,
                crate_roots,
                _lease: lease,
            }),
        })
    }

    pub fn file(&self, index: usize) -> Result<SourceFileHandle, TokenError> {
        self.inner.storage.view(index)?;
        Ok(SourceFileHandle {
            source: self.inner.id,
            index: u32::try_from(index).map_err(|_| TokenError::InvalidRange)?,
        })
    }

    pub fn text(&self, file: SourceFileHandle) -> Result<&str, TokenError> {
        if file.source != self.inner.id {
            return Err(TokenError::ForeignHandle);
        }
        Ok(self.inner.storage.view(file.index as usize)?.contents)
    }

    /// Checked source ownership, issued by assembly rather than inferred from
    /// names, paths, source text or a caller-supplied macro definition handle.
    pub(super) fn source_crate_root(
        &self,
        file: SourceFileHandle,
    ) -> Result<SourceFileHandle, TokenError> {
        self.text(file)?;
        let root = if self.inner.crate_roots.is_empty() {
            0
        } else {
            self.inner
                .crate_roots
                .get(file.index as usize)
                .ok_or(TokenError::InvalidHandle)?
                .0 as usize
        };
        self.file(root)
    }

    pub fn span(
        &self,
        file: SourceFileHandle,
        lo: u32,
        hi: u32,
    ) -> Result<OriginalSpan, TokenError> {
        let text = self.text(file)?;
        if lo > hi || hi as usize > text.len() {
            return Err(TokenError::InvalidRange);
        }
        if !text.is_char_boundary(lo as usize) || !text.is_char_boundary(hi as usize) {
            return Err(TokenError::InvalidUtf8Boundary);
        }
        Ok(OriginalSpan { file, lo, hi })
    }

    pub fn render_span(&self, span: OriginalSpan) -> Result<SourceSpan, TokenError> {
        self.span(span.file, span.lo, span.hi)?;
        Ok(SourceSpan {
            file: FileId(span.file.index),
            lo: span.lo,
            hi: span.hi,
        })
    }

    pub fn location(
        &self,
        span: OriginalSpan,
    ) -> Result<(&std::path::Path, usize, usize), TokenError> {
        let raw = self.render_span(span)?;
        let file = self.inner.storage.view(raw.file.0 as usize)?;
        let line = file
            .line_starts
            .partition_point(|&start| start <= raw.lo)
            .checked_sub(1)
            .ok_or(TokenError::InvalidRange)?;
        let start = file.line_starts[line] as usize;
        let column = file
            .contents
            .get(start..raw.lo as usize)
            .ok_or(TokenError::InvalidRange)?
            .chars()
            .count()
            + 1;
        Ok((file.path, line + 1, column))
    }
}

/// Fallible capacity-accounted flat storage. Payloads must not contain recursive
/// ownership. Reserving capacity charges before allocation; drop refunds bytes only.
pub(super) struct BudgetVec<T> {
    values: Vec<T>,
    lease: Option<ByteLease>,
    budget: ExpansionBudget,
}

impl<T> BudgetVec<T> {
    pub fn new(budget: &ExpansionBudget, storage: Storage) -> Result<Self, TokenError> {
        Ok(Self {
            values: Vec::new(),
            lease: Some(budget.reserve(storage, 0)?),
            budget: budget.clone(),
        })
    }
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
    pub fn len(&self) -> usize {
        self.values.len()
    }
    pub fn last(&self) -> Option<&T> {
        self.values.last()
    }
    pub fn pop(&mut self) -> Option<T> {
        self.values.pop()
    }
    pub fn truncate(&mut self, len: usize) {
        self.values.truncate(len);
    }
    /// In-place sort and duplicate removal. Never allocates or grows the lease.
    pub fn sort_unstable_dedup(&mut self)
    where
        T: Ord,
    {
        self.values.sort_unstable();
        self.values.dedup();
    }
    pub fn push(&mut self, value: T) -> Result<(), TokenError> {
        self.budget.check()?;
        if self.values.len() == self.values.capacity() {
            let target = self
                .values
                .capacity()
                .max(2)
                .checked_mul(2)
                .ok_or(TokenError::IdExhausted)?;
            let bytes = (target as u64)
                .checked_mul(size_of::<T>() as u64)
                .ok_or(TokenError::IdExhausted)?;
            self.lease
                .as_mut()
                .ok_or(TokenError::InvalidHandle)?
                .resize(bytes)?;
            if self
                .values
                .try_reserve_exact(target - self.values.len())
                .is_err()
            {
                return Err(TokenError::Budget(self.budget.poison_allocation()));
            }
            self.reconcile_capacity()?;
        }
        self.values.push(value);
        Ok(())
    }

    /// Allocators may return more capacity than requested. A failed top-up must
    /// not leave that grown backing resident under the smaller planned lease.
    fn reconcile_capacity(&mut self) -> Result<(), TokenError> {
        let result = (self.values.capacity() as u64)
            .checked_mul(size_of::<T>() as u64)
            .ok_or(TokenError::IdExhausted)
            .and_then(|actual| {
                self.lease
                    .as_mut()
                    .ok_or(TokenError::InvalidHandle)?
                    .resize(actual)
                    .map_err(TokenError::from)
            });
        if result.is_err() {
            // Destruction precedes the release of its reservation. Drop works
            // after poison, whereas resize(0) is intentionally not an admission.
            drop(std::mem::take(&mut self.values));
            drop(self.lease.take());
        }
        result
    }
}

struct SymbolEntry {
    hash: u64,
    fingerprint: AtomFingerprint,
    text: String,
    _lease: ByteLease,
}
struct ExpansionEntry {
    record: ExpansionRecord,
    depth: u32,
}

pub struct TokenSession {
    pub(super) fragments: BudgetVec<super::macro_fragments::FragmentPayload>,
    id: SessionId,
    /// Owners whose handles this session admits; exactly `id` today.
    crate_owners: CrateOwners<SessionId>,
    sources: SourceSnapshot,
    budget: ExpansionBudget,
    symbols: BudgetVec<SymbolEntry>,
    symbol_slots: BudgetVec<Option<u32>>,
    spans: BudgetVec<SpanData>,
    contexts: BudgetVec<ContextData>,
    expansions: BudgetVec<ExpansionEntry>,
    crates: BudgetVec<SourceFileHandle>,
    _lease: ByteLease,
}

/// The one-time capability to publish a name store over one session. Holds
/// the session's refundable owner path and the id it is bound to. Not
/// `Clone`/`Copy` and not constructible outside this module: only
/// `TokenSession::with_owner`/`standalone` create one, together with its
/// session. `NameStoreBuilder::new` consumes it and rejects a cap bound to a
/// different session. `TokenSession` itself never holds owner state.
///
/// ```compile_fail
/// fn reuse(cap: justrust_engine::compiler::token_primitives::SessionOwnerCap) {
///     let first = cap;
///     let _second = (cap, first);
/// }
/// ```
#[derive(Debug)]
pub struct SessionOwnerCap {
    session: SessionId,
    path: super::owner_path::BuilderOwnedPath,
}
impl SessionOwnerCap {
    /// Consume the cap for exactly the session it was issued with.
    pub(super) fn bind(
        self,
        session: &TokenSession,
    ) -> Result<super::owner_path::BuilderOwnedPath, TokenError> {
        if self.session != session.id {
            return Err(TokenError::ForeignHandle);
        }
        Ok(self.path)
    }
    /// The builder owner path, for structural checks only.
    pub fn path(&self) -> &super::owner_path::BuilderOwnedPath {
        &self.path
    }
}

impl TokenSession {
    /// A session owned by one issued owner, plus the one-time capability to
    /// publish a store over it. Consumes the `Issued` pair, so one issuance
    /// builds exactly one session. The only production constructor: graph
    /// routes pass the `Issued` from their producer's `OwnerIssuer`.
    pub fn with_owner(
        sources: SourceSnapshot,
        budget: ExpansionBudget,
        owner: super::owner_path::Issued,
    ) -> Result<(Self, SessionOwnerCap), TokenError> {
        // The owner must have been paid by exactly this session's ledger
        // (identity, not equal limits) before any session is built.
        if !owner.path().paid_by(&budget) {
            return Err(TokenError::ForeignHandle);
        }
        let (issuer, tag) = owner.into_parts();
        let id = SessionId(tag.bits());
        if id == ROOT_CONTEXT_OWNER {
            return Err(TokenError::IdExhausted);
        }
        let session = Self::build(id, sources, budget)?;
        Ok((
            session,
            SessionOwnerCap {
                session: id,
                path: issuer.into_path(),
            },
        ))
    }

    /// An independent session for direct, non-graph tooling and tests. Mints
    /// a fresh standalone owner root per call, paid by `budget` and admitted
    /// before anything is minted. Graph code must use `with_owner`.
    pub fn standalone(
        sources: SourceSnapshot,
        budget: ExpansionBudget,
    ) -> Result<(Self, SessionOwnerCap), TokenError> {
        let owner = super::owner_path::standalone_issued(&budget)?;
        Self::with_owner(sources, budget, owner)
    }

    /// Test shorthand: a standalone session whose publish capability is
    /// dropped (refunding it). Such a session cannot back a name store.
    #[cfg(test)]
    pub fn new(sources: SourceSnapshot, budget: ExpansionBudget) -> Result<Self, TokenError> {
        Self::standalone(sources, budget).map(|(session, _)| session)
    }

    fn build(
        id: SessionId,
        sources: SourceSnapshot,
        budget: ExpansionBudget,
    ) -> Result<Self, TokenError> {
        let lease = budget.reserve(Storage::Retained, size_of::<Self>() as u64)?;
        let mut contexts = BudgetVec::new(&budget, Storage::Retained)?;
        budget.charge(Counter::HygieneContexts, 1)?;
        contexts.push(ContextData::Root)?;
        Ok(Self {
            fragments: BudgetVec::new(&budget, Storage::Retained)?,
            _lease: lease,
            id,
            crate_owners: CrateOwners::single(id),
            sources,
            symbols: BudgetVec::new(&budget, Storage::Retained)?,
            symbol_slots: BudgetVec::new(&budget, Storage::Retained)?,
            spans: BudgetVec::new(&budget, Storage::Retained)?,
            contexts,
            expansions: BudgetVec::new(&budget, Storage::Retained)?,
            crates: BudgetVec::new(&budget, Storage::Retained)?,
            budget,
        })
    }
    pub(super) fn id(&self) -> SessionId {
        self.id
    }
    pub fn sources(&self) -> &SourceSnapshot {
        &self.sources
    }
    pub fn budget(&self) -> &ExpansionBudget {
        &self.budget
    }
    /// Whether `owner` is in this session's explicit crate owner set.
    pub(super) fn admits(&self, owner: SessionId) -> bool {
        self.crate_owners.admits(owner)
    }
    fn check_owner(&self, owner: SessionId) -> Result<(), TokenError> {
        self.budget.check()?;
        if !self.admits(owner) {
            Err(TokenError::ForeignHandle)
        } else {
            Ok(())
        }
    }
    fn index(&self, index: usize) -> Result<u32, TokenError> {
        Ok(self.budget.checked_u32(index as u64)?)
    }
    /// The crate-root context is one fixed value shared by every session, so
    /// root-context names from different files compare equal. Mark contexts
    /// stay qualified by their owning session.
    pub fn root_context(&self) -> SyntaxContextId {
        SyntaxContextId::ROOT
    }
    pub fn symbol(&self, id: SymbolId) -> Result<&str, TokenError> {
        self.check_owner(id.session)?;
        self.symbols
            .as_slice()
            .get(id.index as usize)
            .map(|s| s.text.as_str())
            .ok_or(TokenError::InvalidHandle)
    }
    /// Content fingerprint of an interned symbol, equal across sessions.
    pub fn fingerprint(&self, id: SymbolId) -> Result<AtomFingerprint, TokenError> {
        self.check_owner(id.session)?;
        self.symbols
            .as_slice()
            .get(id.index as usize)
            .map(|s| s.fingerprint)
            .ok_or(TokenError::InvalidHandle)
    }
    /// Text of an atom this session interned. A fingerprint the session never
    /// interned is foreign: atoms grant no text authority by themselves.
    pub fn fingerprint_text(&self, fingerprint: AtomFingerprint) -> Result<&str, TokenError> {
        self.budget.check()?;
        let count = self.symbol_slots.len();
        if count == 0 {
            return Err(TokenError::ForeignHandle);
        }
        let mut slot = fingerprint.table_hash() as usize & (count - 1);
        loop {
            self.budget.charge(Counter::Work, 1)?;
            let Some(index) = self.symbol_slots.as_slice()[slot] else {
                return Err(TokenError::ForeignHandle);
            };
            let entry = &self.symbols.as_slice()[index as usize];
            if entry.fingerprint == fingerprint {
                return Ok(entry.text.as_str());
            }
            slot = (slot + 1) & (count - 1);
        }
    }
    /// Exact byte interning keyed by the content fingerprint. Distinct
    /// spellings sharing a fingerprint are rejected, never aliased.
    pub fn intern_symbol(&mut self, text: &str) -> Result<SymbolId, TokenError> {
        self.budget.check()?;
        self.budget.charge(Counter::Work, text.len() as u64 + 1)?;
        let fingerprint = AtomFingerprint::of(text);
        self.intern_fingerprinted(text, fingerprint.table_hash(), fingerprint)
    }
    #[cfg(test)]
    fn intern_hashed(&mut self, text: &str, hash: u64) -> Result<SymbolId, TokenError> {
        self.intern_fingerprinted(text, hash, AtomFingerprint::of(text))
    }

    fn symbol_slot(
        &self,
        text: &str,
        hash: u64,
        fingerprint: AtomFingerprint,
    ) -> Result<Option<(usize, Option<u32>)>, TokenError> {
        let count = self.symbol_slots.len();
        if count == 0 {
            return Ok(None);
        }
        let mut slot = hash as usize & (count - 1);
        loop {
            self.budget.charge(Counter::Work, 1)?;
            let Some(index) = self.symbol_slots.as_slice()[slot] else {
                return Ok(Some((slot, None)));
            };
            let entry = &self.symbols.as_slice()[index as usize];
            if entry.hash == hash {
                self.budget
                    .charge(Counter::Work, entry.text.len().max(text.len()) as u64 + 1)?;
                if entry.text == text {
                    return Ok(Some((slot, Some(index))));
                }
            }
            if entry.fingerprint == fingerprint {
                return Err(TokenError::AtomCollision);
            }
            slot = (slot + 1) & (count - 1);
        }
    }

    fn grow_symbol_index(&mut self) -> Result<(), TokenError> {
        let count = self
            .symbol_slots
            .len()
            .checked_mul(2)
            .ok_or(TokenError::IdExhausted)?
            .max(16);
        // Initialization plus the geometric backing moves while filling this
        // independently leased table are bounded by twice its slot count.
        self.budget.charge(
            Counter::Work,
            (count as u64)
                .checked_mul(2)
                .ok_or(TokenError::IdExhausted)?,
        )?;
        let mut slots = BudgetVec::new(&self.budget, Storage::Retained)?;
        for _ in 0..count {
            slots.push(None)?;
        }
        for (index, entry) in self.symbols.as_slice().iter().enumerate() {
            let mut slot = entry.hash as usize & (count - 1);
            loop {
                self.budget.charge(Counter::Work, 1)?;
                if slots.as_slice()[slot].is_none() {
                    break;
                }
                slot = (slot + 1) & (count - 1);
            }
            slots.values[slot] = Some(self.index(index)?);
        }
        self.symbol_slots = slots;
        Ok(())
    }

    fn intern_fingerprinted(
        &mut self,
        text: &str,
        hash: u64,
        fingerprint: AtomFingerprint,
    ) -> Result<SymbolId, TokenError> {
        self.budget.check()?;
        if let Some((_, Some(index))) = self.symbol_slot(text, hash, fingerprint)? {
            return Ok(SymbolId {
                session: self.id,
                index,
            });
        }
        if self.symbols.len() >= self.symbol_slots.len() / 2 {
            self.grow_symbol_index()?;
        }
        let (slot, _) = self
            .symbol_slot(text, hash, fingerprint)?
            .ok_or(TokenError::InvalidHandle)?;
        let index = self.index(self.symbols.len())?;
        self.budget.charge(Counter::Work, text.len() as u64)?;
        let mut lease = self.budget.reserve(Storage::Retained, text.len() as u64)?;
        let mut owned = String::new();
        if owned.try_reserve_exact(text.len()).is_err() {
            return Err(TokenError::Budget(self.budget.poison_allocation()));
        }
        lease.resize(owned.capacity() as u64)?;
        owned.push_str(text);
        if self.symbols.len() == self.symbols.values.capacity() {
            self.budget
                .charge(Counter::Work, self.symbols.len() as u64)?;
        }
        self.symbols.push(SymbolEntry {
            hash,
            fingerprint,
            text: owned,
            _lease: lease,
        })?;
        self.symbol_slots.values[slot] = Some(index);
        Ok(SymbolId {
            session: self.id,
            index,
        })
    }
    pub fn span(&self, id: SpanId) -> Result<&SpanData, TokenError> {
        self.check_owner(id.session)?;
        self.spans
            .as_slice()
            .get(id.index as usize)
            .ok_or(TokenError::InvalidHandle)
    }
    fn push_span(&mut self, data: SpanData) -> Result<SpanId, TokenError> {
        self.context(data.context)?;
        if let Some(expansion) = data.expansion {
            self.expansion(expansion)?;
        }
        match data.origin {
            SpanOrigin::Original(source) => {
                self.sources.render_span(source)?;
            }
            SpanOrigin::Synthetic { anchor, .. } => {
                self.span(anchor)?;
            }
        }
        let index = self.index(self.spans.len())?;
        self.budget.charge(Counter::SpanRecords, 1)?;
        self.spans.push(data)?;
        Ok(SpanId {
            session: self.id,
            index,
        })
    }
    pub fn original_span(&mut self, source: OriginalSpan) -> Result<SpanId, TokenError> {
        self.push_span(SpanData {
            origin: SpanOrigin::Original(source),
            context: self.root_context(),
            expansion: None,
        })
    }
    pub fn synthetic_span(
        &mut self,
        anchor: SpanId,
        reason: SyntheticReason,
    ) -> Result<SpanId, TokenError> {
        let original = *self.span(anchor)?;
        self.push_span(SpanData {
            origin: SpanOrigin::Synthetic { anchor, reason },
            context: original.context,
            expansion: original.expansion,
        })
    }
    /// Creates a fresh occurrence, preserving the physical origin exactly.
    pub fn with_context(
        &mut self,
        span: SpanId,
        context: SyntaxContextId,
        expansion: Option<ExpansionId>,
    ) -> Result<SpanId, TokenError> {
        let origin = self.span(span)?.origin;
        self.push_span(SpanData {
            origin,
            context,
            expansion,
        })
    }
    /// Fresh expansion occurrence with an explicit backward edge to its input.
    pub fn reissue_span(
        &mut self,
        previous: SpanId,
        context: SyntaxContextId,
        current_expansion: ExpansionId,
    ) -> Result<SpanId, TokenError> {
        self.span(previous)?;
        self.push_span(SpanData {
            origin: SpanOrigin::Synthetic {
                anchor: previous,
                reason: SyntheticReason::ExpansionToken,
            },
            context,
            expansion: Some(current_expansion),
        })
    }
    pub fn context(&self, id: SyntaxContextId) -> Result<&ContextData, TokenError> {
        if id == SyntaxContextId::ROOT {
            self.budget.check()?;
            return Ok(&ContextData::Root);
        }
        self.check_owner(id.session)?;
        self.contexts
            .as_slice()
            .get(id.index as usize)
            .ok_or(TokenError::InvalidHandle)
    }
    pub fn apply_mark(
        &mut self,
        parent: SyntaxContextId,
        expansion: ExpansionId,
        transparency: Transparency,
    ) -> Result<SyntaxContextId, TokenError> {
        self.context(parent)?;
        self.expansion(expansion)?;
        let data = ContextData::Mark {
            parent,
            expansion,
            transparency,
        };
        for (index, context) in self.contexts.as_slice().iter().enumerate() {
            self.budget.charge(Counter::Work, 1)?;
            if *context == data {
                return Ok(SyntaxContextId {
                    session: self.id,
                    index: self.index(index)?,
                });
            }
        }
        let index = self.index(self.contexts.len())?;
        self.budget.charge(Counter::HygieneContexts, 1)?;
        self.contexts.push(data)?;
        Ok(SyntaxContextId {
            session: self.id,
            index,
        })
    }
    pub fn register_crate_root(&mut self, root: SourceFileHandle) -> Result<CrateId, TokenError> {
        self.sources.text(root)?;
        for (index, candidate) in self.crates.as_slice().iter().enumerate() {
            self.budget.charge(Counter::Work, 1)?;
            if *candidate == root {
                return Ok(CrateId {
                    session: self.id,
                    index: self.index(index)?,
                });
            }
        }
        let index = self.index(self.crates.len())?;
        self.crates.push(root)?;
        Ok(CrateId {
            session: self.id,
            index,
        })
    }
    pub fn crate_root(&self, id: CrateId) -> Result<SourceFileHandle, TokenError> {
        self.check_owner(id.session)?;
        self.crates
            .as_slice()
            .get(id.index as usize)
            .copied()
            .ok_or(TokenError::InvalidHandle)
    }
    pub fn expansion(&self, id: ExpansionId) -> Result<&ExpansionRecord, TokenError> {
        self.check_owner(id.session)?;
        self.expansions
            .as_slice()
            .get(id.index as usize)
            .map(|entry| &entry.record)
            .ok_or(TokenError::InvalidHandle)
    }
    pub fn add_expansion(&mut self, record: ExpansionRecord) -> Result<ExpansionId, TokenError> {
        self.span(record.definition)?;
        self.span(record.call)?;
        self.crate_root(record.defining_crate)?;
        let depth = if let Some(parent) = record.parent {
            self.expansion(parent)?;
            self.expansions.as_slice()[parent.index as usize]
                .depth
                .checked_add(1)
                .ok_or(TokenError::IdExhausted)?
        } else {
            1
        };
        self.budget.check_depth(Depth::Expansion, depth)?;
        let index = self.index(self.expansions.len())?;
        self.budget.charge(Counter::ExpansionRecords, 1)?;
        self.expansions.push(ExpansionEntry { record, depth })?;
        Ok(ExpansionId {
            session: self.id,
            index,
        })
    }
    pub fn original(&self, mut span: SpanId, max_links: u32) -> Result<OriginalSpan, TokenError> {
        for _ in 0..max_links {
            self.budget.charge(Counter::Work, 1)?;
            match self.span(span)?.origin {
                SpanOrigin::Original(original) => return Ok(original),
                SpanOrigin::Synthetic { anchor, .. } => span = anchor,
            }
        }
        // Validate even when the requested traversal allowance is zero.
        self.span(span)?;
        Err(TokenError::TraceTruncated)
    }
    pub fn trace(
        &self,
        first: Option<ExpansionId>,
        max_frames: u32,
    ) -> Result<ExpansionTrace<'_>, TokenError> {
        if let Some(id) = first {
            self.expansion(id)?;
        }
        Ok(ExpansionTrace {
            session: self,
            next: first,
            remaining: max_frames.min(self.budget.limits().backtrace_frames),
            truncated: false,
        })
    }
}

pub struct ExpansionTrace<'a> {
    session: &'a TokenSession,
    next: Option<ExpansionId>,
    remaining: u32,
    truncated: bool,
}
impl ExpansionTrace<'_> {
    pub fn truncated(&self) -> bool {
        self.truncated || (self.remaining == 0 && self.next.is_some())
    }
}
impl Iterator for ExpansionTrace<'_> {
    type Item = Result<TraceFrame, TokenError>;
    fn next(&mut self) -> Option<Self::Item> {
        let id = self.next?;
        if self.remaining == 0 {
            self.truncated = true;
            self.next = None;
            return None;
        }
        self.remaining -= 1;
        let record = self
            .session
            .budget
            .charge(Counter::Work, 1)
            .map_err(TokenError::from)
            .and_then(|()| self.session.expansion(id))
            .copied();
        match record {
            Ok(record) => {
                self.next = record.parent;
                Some(Ok(TraceFrame {
                    expansion: id,
                    record,
                }))
            }
            Err(error) => {
                self.next = None;
                Some(Err(error))
            }
        }
    }
}

#[cfg(test)]
mod owner_defect_tests {
    use super::*;
    use crate::compiler::expansion_budget::{ExpansionBudget, ExpansionLimits};
    use crate::compiler::owner_path::{OwnerDomain, OwnerIssuer, standalone_admission_bytes};

    fn snapshot() -> SourceSnapshot {
        let mut map = SourceMap::default();
        map.add("owner.rs".into(), Arc::from("x"));
        SourceSnapshot::new(map).unwrap()
    }

    /// An owner paid under one budget cannot back a session charged to
    /// another, even when both budgets have equal limits (foreign payer).
    #[test]
    fn owner_paid_by_another_budget_with_equal_policy_rejects() {
        let payer = ExpansionBudget::default();
        let other = ExpansionBudget::default();
        assert_eq!(payer.limits(), other.limits(), "fixture needs equal policy");
        let mut issuer = OwnerIssuer::new(crate::OwnerRoot::standalone().unwrap(), &payer).unwrap();
        let owner = issuer.issue(OwnerDomain::TokenSession).unwrap();
        let minted_before = crate::engine::THREAD_MINTS.with(std::cell::Cell::get);
        assert!(matches!(
            TokenSession::with_owner(snapshot(), other, owner),
            Err(TokenError::ForeignHandle)
        ));
        assert_eq!(
            crate::engine::THREAD_MINTS.with(std::cell::Cell::get),
            minted_before
        );
        // The same issuance under its own payer is accepted.
        let owner = issuer.issue(OwnerDomain::TokenSession).unwrap();
        assert!(TokenSession::with_owner(snapshot(), payer, owner).is_ok());
    }

    /// Defect 1 regression (review of 76eef8201): a bare `Copy` tag let one
    /// issuance build two sessions. `with_owner` now consumes the non-`Clone`
    /// `Issued`, so that replay no longer compiles (compile_fail doc on
    /// `Issued`). Each issuance yields one session with its own path.
    #[test]
    fn one_issuance_cannot_build_two_sessions() {
        let budget = ExpansionBudget::default();
        let mut issuer =
            OwnerIssuer::new(crate::OwnerRoot::standalone().unwrap(), &budget).unwrap();
        let first = issuer.issue(OwnerDomain::TokenSession).unwrap();
        let second = issuer.issue(OwnerDomain::TokenSession).unwrap();
        let (a, a_cap) = TokenSession::with_owner(snapshot(), budget.clone(), first).unwrap();
        let (b, b_cap) = TokenSession::with_owner(snapshot(), budget.clone(), second).unwrap();
        assert_ne!(
            a.id(),
            b.id(),
            "two sessions share one owner: grafts would pass"
        );
        assert_ne!(a_cap.path(), b_cap.path());
        assert!(a_cap.path().descends_from(issuer.path()));
    }

    /// Crossed caps over the same snapshot and the same payer are rejected:
    /// a cap publishes only the session it was issued with.
    #[test]
    fn crossed_caps_with_same_snapshot_and_payer_reject() {
        use crate::compiler::semantic_name::{NameError, NameStoreBuilder};
        let budget = ExpansionBudget::default();
        let shared = snapshot();
        let (a, a_cap) = TokenSession::standalone(shared.clone(), budget.clone()).unwrap();
        let (b, b_cap) = TokenSession::standalone(shared, budget.clone()).unwrap();
        assert!(matches!(
            NameStoreBuilder::new(a, b_cap, super::super::token::Edition::E2024),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            NameStoreBuilder::new(b, a_cap, super::super::token::Edition::E2024),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
    }

    /// A dropped cap refunds its owner lease; a session without its cap cannot
    /// publish a store, so no published owner receipt exists for it.
    #[test]
    fn dropped_cap_refunds_and_never_publishes() {
        let budget = ExpansionBudget::default();
        let before = budget.snapshot().retained_bytes;
        let (session, cap) = TokenSession::standalone(snapshot(), budget.clone()).unwrap();
        let with_cap = budget.snapshot().retained_bytes;
        drop(cap);
        let without_cap = budget.snapshot().retained_bytes;
        assert!(without_cap < with_cap, "dropping the cap refunded nothing");
        drop(session);
        assert_eq!(budget.snapshot().retained_bytes, before);
    }

    /// The published store holds the immutable owner; the builder's lease is
    /// converted exactly once and the session itself holds no owner state.
    #[test]
    fn frozen_store_holds_published_owner_only() {
        use crate::compiler::semantic_name::NameStoreBuilder;
        let budget = ExpansionBudget::default();
        let (session, cap) = TokenSession::standalone(snapshot(), budget.clone()).unwrap();
        let builder =
            NameStoreBuilder::new(session, cap, super::super::token::Edition::E2024).unwrap();
        let reserved = budget.snapshot().retained_bytes;
        let store = builder.freeze().unwrap();
        assert!(store.owner_path().is_some());
        assert!(
            budget.snapshot().retained_bytes <= reserved,
            "freeze charged the owner twice"
        );
    }

    /// Defect 2 regression: an exhausted, zero or poisoned caller budget
    /// rejects before any root, tag or session is created.
    #[test]
    fn exhausted_caller_budget_rejects_standalone_session() {
        let minted = || crate::engine::THREAD_MINTS.with(std::cell::Cell::get);
        let zero = ExpansionBudget::new(ExpansionLimits {
            bytes: 0,
            ..ExpansionLimits::default()
        });
        let before = minted();
        assert!(TokenSession::standalone(snapshot(), zero).is_err());
        assert_eq!(
            minted(),
            before,
            "a tag was minted despite a zero caller budget"
        );
        let short = ExpansionBudget::new(ExpansionLimits {
            bytes: standalone_admission_bytes() - 1,
            ..ExpansionLimits::default()
        });
        let before = minted();
        assert!(TokenSession::standalone(snapshot(), short).is_err());
        assert_eq!(
            minted(),
            before,
            "a tag was minted despite a short caller budget"
        );
        let poisoned = ExpansionBudget::default();
        poisoned.poison_allocation();
        let before = minted();
        assert!(TokenSession::standalone(snapshot(), poisoned).is_err());
        assert_eq!(minted(), before);
    }

    /// Owner storage is charged to the caller's ledger, not a private one.
    #[test]
    fn standalone_owner_is_paid_by_the_caller() {
        let budget = ExpansionBudget::default();
        let before = budget.snapshot().retained_bytes;
        let session = TokenSession::standalone(snapshot(), budget.clone()).unwrap();
        let paid = budget.snapshot().retained_bytes - before;
        assert!(
            paid >= standalone_admission_bytes(),
            "caller paid {paid} bytes, owner receipts cost more"
        );
        drop(session);
    }
}

#[cfg(test)]
mod tests {
    use super::super::expansion_budget::{BudgetFailureKind, BudgetResource, ExpansionLimits};
    use super::super::token::{Edition, Ident, LiteralKind, Spacing, Token, TokenKind};
    use super::*;

    fn sources() -> SourceSnapshot {
        let mut map = SourceMap::default();
        map.add("unicode.rs".into(), Arc::from("α\r\nx"));
        SourceSnapshot::new(map).unwrap()
    }
    fn fixture() -> (TokenSession, SpanId, CrateId) {
        let sources = sources();
        let file = sources.file(0).unwrap();
        let origin = sources.span(file, 0, 2).unwrap();
        let mut session = TokenSession::new(sources, ExpansionBudget::default()).unwrap();
        let span = session.original_span(origin).unwrap();
        let krate = session.register_crate_root(file).unwrap();
        (session, span, krate)
    }
    fn record(span: SpanId, krate: CrateId, parent: Option<ExpansionId>) -> ExpansionRecord {
        ExpansionRecord {
            definition: span,
            call: span,
            parent,
            defining_crate: krate,
            edition: Edition::E2024,
        }
    }

    #[test]
    fn symbol_index_handles_4096_distinct_names_with_bounded_work_and_storage() {
        let mut counters = ExpansionLimits::default().counters;
        counters[Counter::Work as usize] = 500_000;
        let budget = ExpansionBudget::new(ExpansionLimits {
            counters,
            ..ExpansionLimits::default()
        });
        let mut session = TokenSession::new(sources(), budget.clone()).unwrap();
        let shared = session.intern_symbol("i32").unwrap();
        for i in 0..4096 {
            let name = format!("p{i}");
            let id = session.intern_symbol(&name).unwrap();
            assert_eq!(session.symbol(id).unwrap(), name);
            assert_eq!(session.intern_symbol(&name).unwrap(), id);
            assert_eq!(session.intern_symbol("i32").unwrap(), shared);
        }
        assert_eq!(session.symbols.len(), 4097);
        assert!(budget.snapshot().retained_bytes > 0);
        let work = budget.snapshot().counters[Counter::Work as usize];
        drop(session);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().counters[Counter::Work as usize], work);
    }

    #[test]
    fn symbol_index_hash_collisions_never_alias_full_spellings_and_rehash() {
        let budget = ExpansionBudget::default();
        let mut session = TokenSession::new(sources(), budget.clone()).unwrap();
        for i in 0..64 {
            let name = format!("collision_{i}");
            let id = session.intern_hashed(&name, 7).unwrap();
            assert_eq!(id.index as usize, i);
            assert_eq!(session.symbol(id).unwrap(), name);
        }
        for i in 0..64 {
            let name = format!("collision_{i}");
            let id = session.intern_hashed(&name, 7).unwrap();
            assert_eq!(id.index as usize, i);
        }
        assert_eq!(session.symbols.len(), 64);
        drop(session);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn borrowed_source_copy_shares_contents_but_seals_metadata_and_lease() {
        let mut map = SourceMap::default();
        map.add("original.rs".into(), Arc::from("α\r\nx\n"));
        let budget = ExpansionBudget::default();
        let snapshot = SourceSnapshot::copy_borrowed(&map, &budget).unwrap();
        let file = snapshot.file(0).unwrap();
        let SourceStorage::Map(copied) = &snapshot.inner.storage else {
            panic!("map backing")
        };
        assert!(Arc::ptr_eq(
            &map.files[0].contents,
            &copied.files[0].contents
        ));
        assert_ne!(
            map.files[0].line_starts.as_ptr(),
            copied.files[0].line_starts.as_ptr()
        );
        let retained = budget.snapshot().retained_bytes;
        assert!(retained > 0);
        let clone = snapshot.clone();
        assert!(snapshot.same_snapshot(&clone));
        let independent = SourceSnapshot::copy_borrowed(&map, &ExpansionBudget::default()).unwrap();
        assert!(!snapshot.same_snapshot(&independent));
        assert!(matches!(
            independent.text(file),
            Err(TokenError::ForeignHandle)
        ));
        map.files[0].lexical_path = "changed.rs".into();
        map.files[0].line_starts.clear();
        let eof = snapshot.span(file, 6, 6).unwrap();
        assert_eq!(
            snapshot.location(eof).unwrap(),
            (Path::new("original.rs"), 3, 1)
        );
        let session = TokenSession::new(snapshot.clone(), ExpansionBudget::default()).unwrap();
        drop(snapshot);
        drop(clone);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        assert_eq!(session.sources().text(file).unwrap(), "α\r\nx\n");
        drop(session);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert!(budget.snapshot().counters[Counter::Work as usize] > 0);
    }

    #[test]
    fn raw_identity_source_preserves_utf8_eof_and_charges_owned_payload() {
        let budget = ExpansionBudget::default();
        let snapshot =
            SourceSnapshot::from_text(Path::new("not-read.rs"), "α\r\nx\n", &budget).unwrap();
        let file = snapshot.file(0).unwrap();
        assert_eq!(snapshot.text(file).unwrap(), "α\r\nx\n");
        assert!(snapshot.file(1).is_err());
        assert!(matches!(
            snapshot.span(file, 1, 2),
            Err(TokenError::InvalidUtf8Boundary)
        ));
        assert_eq!(
            snapshot
                .location(snapshot.span(file, 6, 6).unwrap())
                .unwrap(),
            (Path::new("not-read.rs"), 3, 1)
        );
        let moved = sources();
        assert!(!snapshot.same_snapshot(&moved));
        assert!(matches!(moved.text(file), Err(TokenError::ForeignHandle)));
        let retained = budget.snapshot().retained_bytes;
        assert!(retained >= 6);
        let clone = snapshot.clone();
        drop(snapshot);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        drop(clone);
        assert_eq!(budget.snapshot().retained_bytes, 0);
        let empty =
            SourceSnapshot::from_text(Path::new("empty.rs"), "", &ExpansionBudget::default())
                .unwrap();
        assert_eq!(
            empty
                .location(empty.span(empty.file(0).unwrap(), 0, 0).unwrap())
                .unwrap()
                .1,
            1
        );
    }

    #[test]
    fn source_constructor_failures_release_partial_payload_and_preserve_work() {
        let mut map = SourceMap::default();
        map.add("a-long-path-for-copy.rs".into(), Arc::from("a\nb\nc\n"));
        let healthy = ExpansionBudget::default();
        let snapshot = SourceSnapshot::copy_borrowed(&map, &healthy).unwrap();
        let bytes = healthy.snapshot().retained_bytes;
        drop(snapshot);
        for cap in [0, 1, bytes / 2, bytes - 1] {
            let budget = ExpansionBudget::new(ExpansionLimits {
                bytes: cap,
                ..ExpansionLimits::default()
            });
            assert!(matches!(
                SourceSnapshot::copy_borrowed(&map, &budget),
                Err(TokenError::Budget(_))
            ));
            assert_eq!(budget.snapshot().retained_bytes, 0);
            assert!(budget.snapshot().counters[Counter::Work as usize] > 0);
        }
        let healthy = ExpansionBudget::default();
        let snapshot =
            SourceSnapshot::from_text(Path::new("raw.rs"), "a\nb\nc\n", &healthy).unwrap();
        let bytes = healthy.snapshot().retained_bytes;
        drop(snapshot);
        for cap in [0, bytes / 2, bytes - 1] {
            let budget = ExpansionBudget::new(ExpansionLimits {
                bytes: cap,
                ..ExpansionLimits::default()
            });
            assert!(matches!(
                SourceSnapshot::from_text(Path::new("raw.rs"), "a\nb\nc\n", &budget),
                Err(TokenError::Budget(_))
            ));
            assert_eq!(budget.snapshot().retained_bytes, 0);
        }
    }

    #[test]
    fn borrowed_metadata_traversal_is_budgeted_before_copy() {
        let mut map = SourceMap::default();
        map.add("large-metadata.rs".into(), Arc::from(""));
        map.files[0].line_starts = vec![0; 128];
        let mut counters = ExpansionLimits::default().counters;
        counters[Counter::Work as usize] = 8;
        let budget = ExpansionBudget::new(ExpansionLimits {
            counters,
            ..ExpansionLimits::default()
        });
        assert!(matches!(
            SourceSnapshot::copy_borrowed(&map, &budget),
            Err(TokenError::Budget(_))
        ));
        assert_eq!(budget.snapshot().retained_bytes, 0);
        map.files[0].line_starts = vec![0];
        map.files[0].lexical_path = "x".repeat(128).into();
        let budget = ExpansionBudget::new(ExpansionLimits {
            counters,
            ..ExpansionLimits::default()
        });
        assert!(matches!(
            SourceSnapshot::copy_borrowed(&map, &budget),
            Err(TokenError::Budget(_))
        ));
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn source_snapshot_validates_owner_utf8_eof_and_line_tables() {
        let sources = sources();
        let file = sources.file(0).unwrap();
        assert_eq!(sources.text(file).unwrap(), "α\r\nx");
        assert!(matches!(
            sources.span(file, 1, 2),
            Err(TokenError::InvalidUtf8Boundary)
        ));
        assert!(matches!(
            sources.span(file, 2, 1),
            Err(TokenError::InvalidRange)
        ));
        let eof = sources.span(file, 5, 5).unwrap();
        assert_eq!(sources.location(eof).unwrap().1, 2);
        assert_eq!(sources.location(eof).unwrap().2, 2);
        assert_eq!(sources.render_span(eof).unwrap().lo, 5);
        let other = self::sources();
        assert!(matches!(
            other.span(file, 0, 2),
            Err(TokenError::ForeignHandle)
        ));
        assert!(matches!(
            other.render_span(eof),
            Err(TokenError::ForeignHandle)
        ));
        assert!(sources.clone().text(file).is_ok());
        let mut malformed = SourceMap::default();
        malformed.add("bad.rs".into(), Arc::from("x\ny"));
        malformed.files[0].line_starts.clear();
        assert!(matches!(
            SourceSnapshot::new(malformed),
            Err(TokenError::InvalidInput(_))
        ));
    }

    #[test]
    fn equal_local_indices_do_not_grant_foreign_session_authority() {
        let (mut a, a_span, _) = fixture();
        let (mut b, b_span, b_crate) = fixture();
        let a_symbol = a.intern_symbol("same").unwrap();
        let b_symbol = b.intern_symbol("same").unwrap();
        assert_ne!(a_symbol, b_symbol);
        assert!(matches!(a.symbol(b_symbol), Err(TokenError::ForeignHandle)));
        assert!(matches!(a.span(b_span), Err(TokenError::ForeignHandle)));
        assert_eq!(a.root_context(), b.root_context());
        assert_eq!(
            a.fingerprint(a_symbol).unwrap(),
            b.fingerprint(b_symbol).unwrap()
        );
        let b_exp = b.add_expansion(record(b_span, b_crate, None)).unwrap();
        let b_root = b.root_context();
        let b_mark = b
            .apply_mark(b_root, b_exp, Transparency::SemiTransparent)
            .unwrap();
        assert!(matches!(a.context(b_mark), Err(TokenError::ForeignHandle)));
        assert!(matches!(
            a.synthetic_span(b_span, SyntheticReason::ExpansionToken),
            Err(TokenError::ForeignHandle)
        ));
        assert!(matches!(
            a.add_expansion(record(a_span, b_crate, None)),
            Err(TokenError::ForeignHandle)
        ));
        let foreign_original = b.original(b_span, 1).unwrap();
        assert!(matches!(
            a.original_span(foreign_original),
            Err(TokenError::ForeignHandle)
        ));
        let forged = SpanId {
            session: a.id,
            index: u32::MAX,
        };
        assert!(matches!(a.span(forged), Err(TokenError::InvalidHandle)));
    }

    #[test]
    fn shared_source_owner_still_has_separate_session_contexts() {
        let sources = sources();
        let file = sources.file(0).unwrap();
        let original = sources.span(file, 0, 2).unwrap();
        let mut a = TokenSession::new(sources.clone(), ExpansionBudget::default()).unwrap();
        let mut b = TokenSession::new(sources, ExpansionBudget::default()).unwrap();
        let span_a = a.original_span(original).unwrap();
        let span_b = b.original_span(original).unwrap();
        assert_ne!(span_a, span_b);
        // The crate-root context is shared; spans stay session-owned.
        a.with_context(span_a, b.root_context(), None).unwrap();
        assert!(matches!(
            a.with_context(span_b, a.root_context(), None),
            Err(TokenError::ForeignHandle)
        ));
    }

    #[test]
    fn atom_fingerprints_are_content_canonical_and_collisions_reject() {
        let (mut a, _, _) = fixture();
        let (mut b, _, _) = fixture();
        let x = a.intern_symbol("value").unwrap();
        let y = b.intern_symbol("value").unwrap();
        assert_eq!(AtomFingerprint::of("value"), a.fingerprint(x).unwrap());
        assert_eq!(a.fingerprint(x).unwrap(), b.fingerprint(y).unwrap());
        assert_eq!(
            a.fingerprint_text(b.fingerprint(y).unwrap()).unwrap(),
            "value"
        );
        assert!(matches!(
            a.fingerprint_text(AtomFingerprint::of("never_interned")),
            Err(TokenError::ForeignHandle)
        ));
        // A forged entry with a colliding fingerprint must never alias.
        let forged = AtomFingerprint::of("value");
        assert!(matches!(
            a.intern_fingerprinted("other", forged.table_hash(), forged),
            Err(TokenError::AtomCollision)
        ));
    }

    #[test]
    fn explicit_root_interned_marks_and_repeated_occurrence_origins() {
        let (mut session, span, krate) = fixture();
        let root = session.root_context();
        assert_eq!(*session.context(root).unwrap(), ContextData::Root);
        let exp = session.add_expansion(record(span, krate, None)).unwrap();
        let mark = session
            .apply_mark(root, exp, Transparency::SemiTransparent)
            .unwrap();
        let before = session.budget().snapshot().counters[Counter::HygieneContexts as usize];
        assert_eq!(
            session
                .apply_mark(root, exp, Transparency::SemiTransparent)
                .unwrap(),
            mark
        );
        assert_eq!(
            session.budget().snapshot().counters[Counter::HygieneContexts as usize],
            before
        );
        let a = session.with_context(span, mark, Some(exp)).unwrap();
        let b = session.with_context(span, root, Some(exp)).unwrap();
        assert_ne!(a, b);
        assert_eq!(
            session.original(a, 1).unwrap(),
            session.original(b, 1).unwrap()
        );
        assert_eq!(session.span(span).unwrap().expansion, None);
        assert_eq!(session.span(a).unwrap().context, mark);
        assert_eq!(session.span(b).unwrap().context, root);
        let symbol = session.intern_symbol("fn").unwrap();
        let raw = Token {
            span: b,
            kind: TokenKind::Ident(Ident {
                symbol,
                raw: true,
                context: root,
            }),
        };
        raw.validate(&session).unwrap(); // Structural only, never classified as a keyword.
        Token {
            span: b,
            kind: TokenKind::Literal {
                kind: LiteralKind::RawString,
                spelling: symbol,
            },
        }
        .validate(&session)
        .unwrap(); // No claim the supplied spelling is lexically valid.
        let bad_context = Token {
            span: a,
            kind: raw.kind,
        };
        assert!(matches!(
            bad_context.validate(&session),
            Err(TokenError::InvalidInput(_))
        ));
        assert!(
            Token {
                span,
                kind: TokenKind::DollarCrate(krate)
            }
            .validate(&session)
            .is_ok()
        );
        assert!(
            Token {
                span,
                kind: TokenKind::Punct {
                    ch: '{',
                    spacing: Spacing::Joint
                }
            }
            .validate(&session)
            .is_err()
        );
    }

    #[test]
    fn bounded_parent_trace_and_synthetic_anchor_do_not_recurse() {
        let (mut session, span, krate) = fixture();
        let mut parent = None;
        for _ in 0..80 {
            parent = Some(session.add_expansion(record(span, krate, parent)).unwrap());
        }
        let mut trace = session.trace(parent, u32::MAX).unwrap();
        let mut frames = 0;
        for frame in trace.by_ref() {
            frame.unwrap();
            frames += 1;
        }
        assert_eq!(frames, 64);
        assert!(trace.truncated());
        let mut empty = session.trace(parent, 0).unwrap();
        assert!(empty.next().is_none());
        assert!(empty.truncated());
        let mut anchor = span;
        for _ in 0..300 {
            anchor = session
                .synthetic_span(anchor, SyntheticReason::TransportDelimiter)
                .unwrap();
        }
        assert!(matches!(
            session.original(anchor, 64),
            Err(TokenError::TraceTruncated)
        ));
        assert_eq!(
            session.original(anchor, 301).unwrap(),
            session.original(span, 1).unwrap()
        );
        assert_eq!(
            session
                .sources()
                .location(session.original(anchor, 301).unwrap())
                .unwrap()
                .2,
            1
        );
    }

    #[test]
    fn exact_expansion_depth_limit_and_typed_failure_latching() {
        let (mut session, span, krate) = fixture();
        let mut parent = None;
        for _ in 0..128 {
            parent = Some(session.add_expansion(record(span, krate, parent)).unwrap());
        }
        let Err(TokenError::Budget(error)) = session.add_expansion(record(span, krate, parent))
        else {
            panic!("depth rejection required");
        };
        assert_eq!(error.kind, BudgetFailureKind::Limit);
        assert_eq!(error.resource, BudgetResource::Depth(Depth::Expansion));
        let Err(TokenError::Budget(again)) = session.intern_symbol("after failure") else {
            panic!("latched rejection required");
        };
        assert_eq!(error, again);
    }

    #[test]
    fn zero_storage_and_allocated_payload_release_preserve_monotonic_counts() {
        let limits = ExpansionLimits {
            bytes: 0,
            ..ExpansionLimits::default()
        };
        let budget = ExpansionBudget::new(limits);
        assert!(matches!(
            TokenSession::new(sources(), budget.clone()),
            Err(TokenError::Budget(_))
        ));
        assert_eq!(budget.snapshot().retained_bytes, 0);
        let budget = ExpansionBudget::default();
        {
            let mut session = TokenSession::new(sources(), budget.clone()).unwrap();
            session.intern_symbol("exact spelling").unwrap();
            assert_eq!(
                session.intern_symbol("exact spelling").unwrap(),
                session.intern_symbol("exact spelling").unwrap()
            );
            assert!(budget.snapshot().retained_bytes > 0);
        }
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
        assert!(budget.snapshot().counters[Counter::HygieneContexts as usize] > 0);
    }

    #[test]
    fn checked_identity_counter_never_wraps_or_reuses() {
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(allocate_identity(&counter).unwrap().get(), u64::MAX - 1);
        assert!(matches!(
            allocate_identity(&counter),
            Err(TokenError::IdExhausted)
        ));
        assert!(matches!(
            allocate_identity(&counter),
            Err(TokenError::IdExhausted)
        ));
        assert_eq!(counter.load(Ordering::Relaxed), u64::MAX);
    }

    #[test]
    fn source_import_file_boundary_and_preflight_precede_line_scan() {
        let mut exact = SourceMap::default();
        for _ in 0..MAX_FILES {
            exact.add("small.rs".into(), Arc::from(""));
        }
        assert!(SourceSnapshot::new(exact).is_ok());
        let mut above = SourceMap::default();
        for _ in 0..=MAX_FILES {
            above.add("small.rs".into(), Arc::from(""));
        }
        above.files[0].line_starts.clear();
        assert!(
            matches!(SourceSnapshot::new(above), Err(TokenError::SourceLimit {
            resource: SourceLimitResource::Files, used: 0, requested, limit
        }) if requested == MAX_FILES + 1 && limit == MAX_FILES)
        );
    }

    #[test]
    fn source_import_byte_boundary_aggregate_and_preflight() {
        let contents: Arc<str> = Arc::from("x".repeat(MAX_BYTES));
        let mut exact = SourceMap::default();
        exact.add("exact.rs".into(), contents.clone());
        assert!(SourceSnapshot::new(exact).is_ok());
        let mut above = SourceMap::default();
        above.add("exact.rs".into(), contents);
        above.add("extra.rs".into(), Arc::from("x"));
        above.files[0].line_starts.clear();
        assert!(
            matches!(SourceSnapshot::new(above), Err(TokenError::SourceLimit {
            resource: SourceLimitResource::Bytes, used, requested: 1, limit
        }) if used == MAX_BYTES && limit == MAX_BYTES)
        );
        let mut single = SourceMap::default();
        single.add("large.rs".into(), Arc::from("x".repeat(MAX_BYTES + 1)));
        assert!(
            matches!(SourceSnapshot::new(single), Err(TokenError::SourceLimit {
            resource: SourceLimitResource::Bytes, used: 0, requested, limit
        }) if requested == MAX_BYTES + 1 && limit == MAX_BYTES)
        );
    }

    #[test]
    fn rounded_capacity_failure_frees_backing_and_lease_before_owner_drop() {
        let limits = ExpansionLimits {
            bytes: 6,
            ..ExpansionLimits::default()
        };
        let budget = ExpansionBudget::new(limits);
        let mut values = BudgetVec::<u8>::new(&budget, Storage::Retained).unwrap();
        values.lease.as_mut().unwrap().resize(4).unwrap();
        // Direct private fixture models an allocator returning capacity 8 for a
        // planned capacity 4. Production reaches this helper after try_reserve.
        values.values = Vec::with_capacity(8);
        values.values.push(7);
        let Err(TokenError::Budget(failure)) = values.reconcile_capacity() else {
            panic!("rounded capacity must reject");
        };
        assert_eq!(failure.kind, BudgetFailureKind::Limit);
        assert_eq!(failure.resource, BudgetResource::Bytes);
        assert_eq!(values.values.capacity(), 0);
        assert!(values.lease.is_none());
        assert_eq!(budget.snapshot().retained_bytes, 0);
        assert_eq!(budget.check(), Err(failure.clone()));
        assert!(matches!(values.push(9), Err(TokenError::Budget(same)) if same == failure));
        drop(values);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn healthy_rounded_capacity_keeps_payload_and_accounts_actual_capacity() {
        let budget = ExpansionBudget::default();
        let mut values = BudgetVec::<u8>::new(&budget, Storage::Temporary).unwrap();
        values.lease.as_mut().unwrap().resize(4).unwrap();
        values.values = Vec::with_capacity(8);
        values.values.push(7);
        values.reconcile_capacity().unwrap();
        assert_eq!(values.as_slice(), &[7]);
        assert_eq!(
            budget.snapshot().temporary_bytes,
            values.values.capacity() as u64
        );
        drop(values);
        assert_eq!(budget.snapshot().temporary_bytes, 0);
    }
}
