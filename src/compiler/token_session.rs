//! Sealed source ownership and issuing authority for token-only handles.
//! No raw FileId/SourceSpan is accepted as an owned source reference.
use std::mem::size_of;
use std::num::NonZeroU64;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use super::expansion_budget::{BudgetFailure, ByteLease, Counter, Depth, ExpansionBudget, Storage};
use super::expansion_span::{ExpansionRecord, SpanData, SpanOrigin, SyntheticReason, TraceFrame};
use super::hygiene::{ContextData, Transparency};
use super::source::source_map::{FileId, SourceMap, SourceSpan};
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

macro_rules! session_handle {
    ($($name:ident),+ $(,)?) => {$(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub struct $name { session: SessionId, index: u32 }
    )+};
}
session_handle!(SpanId, ExpansionId, SyntaxContextId, SymbolId, CrateId);

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
    map: SourceMap,
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
        for file in &map.files {
            u32::try_from(file.contents.len()).map_err(|_| TokenError::InvalidRange)?;
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
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = SourceSnapshotId(allocate_identity(&NEXT)?);
        Ok(Self {
            inner: Arc::new(SealedSources { id, map }),
        })
    }

    pub fn file(&self, index: usize) -> Result<SourceFileHandle, TokenError> {
        self.inner
            .map
            .files
            .get(index)
            .ok_or(TokenError::InvalidHandle)?;
        Ok(SourceFileHandle {
            source: self.inner.id,
            index: u32::try_from(index).map_err(|_| TokenError::InvalidRange)?,
        })
    }

    pub fn text(&self, file: SourceFileHandle) -> Result<&str, TokenError> {
        if file.source != self.inner.id {
            return Err(TokenError::ForeignHandle);
        }
        self.inner
            .map
            .files
            .get(file.index as usize)
            .map(|f| f.contents.as_ref())
            .ok_or(TokenError::InvalidHandle)
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
        self.inner.map.location(raw).ok_or(TokenError::InvalidRange)
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
    text: String,
    _lease: ByteLease,
}
struct ExpansionEntry {
    record: ExpansionRecord,
    depth: u32,
}

pub struct TokenSession {
    id: SessionId,
    sources: SourceSnapshot,
    budget: ExpansionBudget,
    symbols: BudgetVec<SymbolEntry>,
    spans: BudgetVec<SpanData>,
    contexts: BudgetVec<ContextData>,
    expansions: BudgetVec<ExpansionEntry>,
    crates: BudgetVec<SourceFileHandle>,
    _lease: ByteLease,
}

impl TokenSession {
    pub fn new(sources: SourceSnapshot, budget: ExpansionBudget) -> Result<Self, TokenError> {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let id = SessionId(allocate_identity(&NEXT)?);
        let lease = budget.reserve(Storage::Retained, size_of::<Self>() as u64)?;
        let mut contexts = BudgetVec::new(&budget, Storage::Retained)?;
        budget.charge(Counter::HygieneContexts, 1)?;
        contexts.push(ContextData::Root)?;
        Ok(Self {
            _lease: lease,
            id,
            sources,
            symbols: BudgetVec::new(&budget, Storage::Retained)?,
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
    fn check_owner(&self, owner: SessionId) -> Result<(), TokenError> {
        self.budget.check()?;
        if owner != self.id {
            Err(TokenError::ForeignHandle)
        } else {
            Ok(())
        }
    }
    fn index(&self, index: usize) -> Result<u32, TokenError> {
        Ok(self.budget.checked_u32(index as u64)?)
    }
    pub fn root_context(&self) -> SyntaxContextId {
        SyntaxContextId {
            session: self.id,
            index: 0,
        }
    }
    pub fn symbol(&self, id: SymbolId) -> Result<&str, TokenError> {
        self.check_owner(id.session)?;
        self.symbols
            .as_slice()
            .get(id.index as usize)
            .map(|s| s.text.as_str())
            .ok_or(TokenError::InvalidHandle)
    }
    /// Exact byte interning only, with bounded linear lookup for this first slice.
    pub fn intern_symbol(&mut self, text: &str) -> Result<SymbolId, TokenError> {
        self.budget.check()?;
        self.budget.charge(Counter::Work, text.len() as u64 + 1)?;
        for (index, entry) in self.symbols.as_slice().iter().enumerate() {
            self.budget
                .charge(Counter::Work, entry.text.len().max(text.len()) as u64 + 1)?;
            if entry.text == text {
                return Ok(SymbolId {
                    session: self.id,
                    index: self.index(index)?,
                });
            }
        }
        let index = self.index(self.symbols.len())?;
        let mut lease = self.budget.reserve(Storage::Retained, text.len() as u64)?;
        let mut owned = String::new();
        if owned.try_reserve_exact(text.len()).is_err() {
            return Err(TokenError::Budget(self.budget.poison_allocation()));
        }
        lease.resize(owned.capacity() as u64)?;
        owned.push_str(text);
        self.symbols.push(SymbolEntry {
            text: owned,
            _lease: lease,
        })?;
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
    pub fn context(&self, id: SyntaxContextId) -> Result<&ContextData, TokenError> {
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
        assert!(matches!(
            a.context(b.root_context()),
            Err(TokenError::ForeignHandle)
        ));
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
        assert!(matches!(
            a.with_context(span_a, b.root_context(), None),
            Err(TokenError::ForeignHandle)
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
