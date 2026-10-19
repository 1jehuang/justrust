//! Shared structural source paths and semantic keys, never printable identities.
//! Name access is sealed to the two checked owners. Resolver maps must pair a
//! key with their typed lexical/module scope and precharge key traversal work.
use std::fmt;
use std::hash::{Hash, Hasher};
use std::mem::size_of;
use std::ops::Range;
use std::sync::Arc;

use super::expansion_budget::{ByteLease, Counter, ExpansionBudget, Storage};
use super::semantic_name::{
    AstIdent, LookupClass, NameError, NameKey, NameStore, NameStoreBuilder, Namespace,
};
use super::token_session::{BudgetVec, SessionId, SpanId, TokenError, TokenSession};

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::NameStoreBuilder {}
    impl Sealed for super::NameStore {}
}

/// Borrowed access before or after the program name arena is frozen.
pub trait NameAccess: sealed::Sealed {
    fn check(&self, ident: AstIdent) -> Result<(), NameError>;
    fn key(&self, ident: AstIdent, class: LookupClass) -> Result<NameKey, NameError>;
    fn spelling(&self, ident: AstIdent) -> Result<&str, NameError>;
    fn span(&self, ident: AstIdent) -> Result<SpanId, NameError>;
    fn budget(&self) -> &ExpansionBudget;
    fn session(&self) -> &TokenSession;
}
macro_rules! name_access {
    ($ty:ty) => {
        impl NameAccess for $ty {
            fn check(&self, ident: AstIdent) -> Result<(), NameError> {
                <$ty>::check(self, ident)
            }
            fn key(&self, ident: AstIdent, class: LookupClass) -> Result<NameKey, NameError> {
                <$ty>::key(self, ident, class)
            }
            fn spelling(&self, ident: AstIdent) -> Result<&str, NameError> {
                <$ty>::spelling(self, ident)
            }
            fn span(&self, ident: AstIdent) -> Result<SpanId, NameError> {
                <$ty>::span(self, ident)
            }
            fn budget(&self) -> &ExpansionBudget {
                <$ty>::budget(self)
            }
            fn session(&self) -> &TokenSession {
                <$ty>::session(self)
            }
        }
    };
}
name_access!(NameStoreBuilder);
name_access!(NameStore);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PathAnchor {
    Relative,
    Crate,
    SelfModule,
    Super(u16),
    SelfType,
}

/// An authenticated parser-retained `self` token. Public ASTs may transport or
/// transplant it, but cannot fabricate one from an unrelated identifier.
#[derive(Clone, Copy, Debug)]
pub struct ReceiverOccurrence(AstIdent);
impl ReceiverOccurrence {
    pub(crate) fn ident(self) -> AstIdent {
        self.0
    }
}

struct PathData {
    owner: SessionId,
    receiver: Option<ReceiverOccurrence>,
    segments: BudgetVec<AstIdent>,
    keys: BudgetVec<NameKey>,
    // Released only after both flat payloads, including on the final Arc drop.
    _lease: ByteLease,
}

// This is a resolver-issued transport certificate, not a name or lookup key.
// The two payloads are flat shared paths. Re-resolution collapses to the first
// use site rather than constructing a recursive chain of certificates.
struct ResolvedUse {
    target_anchor: PathAnchor,
    target: Arc<PathData>,
    original_anchor: PathAnchor,
    original: Arc<PathData>,
    _lease: ByteLease,
}

/// A syntactic occurrence path. Semantic equality is available through PathKey,
/// never through equality of occurrence handles or displayed source text.
#[derive(Clone)]
pub struct AstPath {
    anchor: PathAnchor,
    data: Arc<PathData>,
    resolved_use: Option<Arc<ResolvedUse>>,
}
impl fmt::Debug for AstPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AstPath")
            .field("anchor", &self.anchor)
            .field("owner", &self.data.owner)
            .field("segments", &self.len())
            .finish()
    }
}

/// Allocation-free key creation shares the same immutable path data. Equality
/// and hashing are pure even after budget poison, as required by their traits.
/// The resolver must precharge traversal before each map lookup or insertion.
#[derive(Clone)]
pub struct PathKey {
    anchor: PathAnchor,
    namespace: Namespace,
    data: Arc<PathData>,
}
impl PartialEq for PathKey {
    fn eq(&self, other: &Self) -> bool {
        self.data.owner == other.data.owner
            && self.anchor == other.anchor
            && self.namespace == other.namespace
            && self.data.keys.as_slice() == other.data.keys.as_slice()
    }
}
impl Eq for PathKey {}
impl Hash for PathKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.data.owner.hash(state);
        self.anchor.hash(state);
        self.namespace.hash(state);
        self.data.keys.as_slice().hash(state);
    }
}
impl fmt::Debug for PathKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PathKey")
            .field("anchor", &self.anchor)
            .field("owner", &self.data.owner)
            .field("namespace", &self.namespace)
            .field("segments", &self.data.keys.len())
            .finish()
    }
}

/// Owned diagnostic text whose actual backing remains accounted until drop.
/// Formatting into another retained diagnostic requires the caller's own budget.
pub struct PathDisplay {
    text: String,
    _lease: ByteLease,
}
impl PathDisplay {
    pub fn as_str(&self) -> &str {
        &self.text
    }
}
impl fmt::Display for PathDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}
impl fmt::Debug for PathDisplay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("PathDisplay").field(&self.as_str()).finish()
    }
}

impl AstPath {
    pub fn new(
        names: &impl NameAccess,
        anchor: PathAnchor,
        segments: &[AstIdent],
    ) -> Result<Self, NameError> {
        Self::build(names, anchor, segments.len(), segments.iter().copied())
    }

    pub(crate) fn receiver(
        names: &impl NameAccess,
        occurrence: AstIdent,
    ) -> Result<Self, NameError> {
        names.check(occurrence)?;
        if names.spelling(occurrence)? != "self" {
            return Err(TokenError::InvalidInput("receiver occurrence is not self").into());
        }
        let mut path = Self::new(names, PathAnchor::SelfModule, &[])?;
        Arc::get_mut(&mut path.data)
            .ok_or(TokenError::InvalidHandle)?
            .receiver = Some(ReceiverOccurrence(occurrence));
        Ok(path)
    }

    pub(crate) fn receiver_occurrence(&self) -> Option<ReceiverOccurrence> {
        self.data.receiver
    }

    fn build(
        names: &impl NameAccess,
        anchor: PathAnchor,
        count: usize,
        input: impl Iterator<Item = AstIdent>,
    ) -> Result<Self, NameError> {
        let budget = names.budget();
        budget.check()?;
        budget.checked_u32(count as u64)?;
        // Two flat geometric vectors: one write plus amortized relocation <=3
        // for each vector element. Individual checked name/key reads also charge.
        let work = (count as u64)
            .checked_mul(6)
            .and_then(|n| n.checked_add(1))
            .ok_or(TokenError::IdExhausted)?;
        budget.charge(Counter::Work, work)?;
        let fixed = size_of::<PathData>()
            .checked_add(2 * size_of::<usize>())
            .ok_or(TokenError::IdExhausted)?;
        let lease = budget.reserve(Storage::Retained, fixed as u64)?;
        let mut segments = BudgetVec::new(budget, Storage::Retained)?;
        let mut keys = BudgetVec::new(budget, Storage::Retained)?;
        for ident in input {
            names.check(ident)?;
            let key = names.key(ident, LookupClass::Item(Namespace::Type))?;
            segments.push(ident)?;
            keys.push(key)?;
        }
        if segments.len() != count {
            return Err(TokenError::InvalidRange.into());
        }
        Ok(Self {
            anchor,
            data: Arc::new(PathData {
                owner: names.session().id(),
                receiver: None,
                segments,
                keys,
                _lease: lease,
            }),
            resolved_use: None,
        })
    }

    pub fn check(&self, names: &impl NameAccess) -> Result<(), NameError> {
        names.budget().check()?;
        names.budget().charge(Counter::Work, 1)?;
        if self.data.owner != names.session().id() {
            return Err(TokenError::ForeignHandle.into());
        }
        if let Some(origin) = &self.resolved_use
            && (origin.target_anchor != self.anchor
                || !Arc::ptr_eq(&origin.target, &self.data)
                || origin.original.owner != self.data.owner)
        {
            return Err(TokenError::InvalidInput("resolved path use certificate mismatch").into());
        }
        if let Some(receiver) = self.data.receiver {
            names.check(receiver.0)?;
            if self.anchor != PathAnchor::SelfModule
                || !self.is_empty()
                || names.spelling(receiver.0)? != "self"
            {
                return Err(TokenError::InvalidInput("receiver path shape changed").into());
            }
        }
        for ident in self.segments() {
            names.check(*ident)?;
        }
        Ok(())
    }

    /// Attach original occurrence authority after semantic resolution. Only the
    /// compiler's resolver can issue this certificate. Semantic keys continue to
    /// use the target, while admission checks inspect the actual use site.
    pub(crate) fn with_resolved_use(
        &self,
        names: &impl NameAccess,
        source: &AstPath,
    ) -> Result<Self, NameError> {
        self.check(names)?;
        source.check(names)?;
        let (original_anchor, original) = match &source.resolved_use {
            Some(origin) => (origin.original_anchor, origin.original.clone()),
            None => (source.anchor, source.data.clone()),
        };
        let fixed = size_of::<ResolvedUse>()
            .checked_add(2 * size_of::<usize>())
            .ok_or(TokenError::IdExhausted)?;
        let lease = names.budget().reserve(Storage::Retained, fixed as u64)?;
        Ok(Self {
            anchor: self.anchor,
            data: self.data.clone(),
            resolved_use: Some(Arc::new(ResolvedUse {
                target_anchor: self.anchor,
                target: self.data.clone(),
                original_anchor,
                original,
                _lease: lease,
            })),
        })
    }

    pub(crate) fn source_occurrences(
        &self,
        names: &impl NameAccess,
    ) -> Result<&[AstIdent], NameError> {
        self.check(names)?;
        Ok(match &self.resolved_use {
            Some(origin) => match &origin.original.receiver {
                Some(receiver) => std::slice::from_ref(&receiver.0),
                None => origin.original.segments.as_slice(),
            },
            None => match &self.data.receiver {
                Some(receiver) => std::slice::from_ref(&receiver.0),
                None => self.segments(),
            },
        })
    }

    pub(crate) fn original_use(&self, names: &impl NameAccess) -> Result<Self, NameError> {
        self.check(names)?;
        Ok(match &self.resolved_use {
            Some(origin) => Self {
                anchor: origin.original_anchor,
                data: origin.original.clone(),
                resolved_use: None,
            },
            None => self.clone(),
        })
    }

    /// Public mutable generic segments may not rebind a resolver certificate to
    /// a different target, even when both occurrences have the same NameKey.
    pub(crate) fn check_target(
        &self,
        names: &impl NameAccess,
        target: &AstPath,
    ) -> Result<(), NameError> {
        self.check(names)?;
        target.check(names)?;
        if self.anchor != target.anchor || self.len() != target.len() {
            return Err(TokenError::InvalidInput("resolved generic target mismatch").into());
        }
        for (expected, actual) in self.segments().iter().zip(target.segments()) {
            if super::semantic_name::occurrence_index(names, *expected)?
                != super::semantic_name::occurrence_index(names, *actual)?
            {
                return Err(TokenError::InvalidInput(
                    "resolved generic target occurrence mismatch",
                )
                .into());
            }
        }
        Ok(())
    }

    fn check_structural_edit(&self) -> Result<(), NameError> {
        if self.resolved_use.is_some() || self.data.receiver.is_some() {
            return Err(
                TokenError::InvalidInput("cannot structurally edit a resolved use path").into(),
            );
        }
        Ok(())
    }

    /// Semantic registry query only. The result is not a source-use occurrence
    /// and must not replace an AST use. Public structural editing stays closed
    /// for resolver-certified paths.
    pub(crate) fn semantic_slice(
        &self,
        names: &impl NameAccess,
        range: Range<usize>,
        anchor: PathAnchor,
    ) -> Result<Self, NameError> {
        self.check(names)?;
        Self::new(
            names,
            anchor,
            self.segments().get(range).ok_or(TokenError::InvalidRange)?,
        )
    }

    pub(crate) fn semantic_parent(&self, names: &impl NameAccess) -> Result<Self, NameError> {
        self.semantic_slice(names, 0..self.len().saturating_sub(1), self.anchor)
    }
    pub fn segments(&self) -> &[AstIdent] {
        self.data.segments.as_slice()
    }
    pub fn anchor(&self) -> PathAnchor {
        self.anchor
    }
    pub fn len(&self) -> usize {
        self.data.segments.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn single(&self) -> Option<AstIdent> {
        (self.anchor == PathAnchor::Relative && self.len() == 1).then(|| self.segments()[0])
    }
    pub fn key(&self, namespace: Namespace) -> PathKey {
        PathKey {
            anchor: self.anchor,
            namespace,
            data: self.data.clone(),
        }
    }
    pub fn push(&self, names: &impl NameAccess, ident: AstIdent) -> Result<Self, NameError> {
        self.check(names)?;
        self.check_structural_edit()?;
        let count = self.len().checked_add(1).ok_or(TokenError::IdExhausted)?;
        Self::build(
            names,
            self.anchor,
            count,
            self.segments()
                .iter()
                .copied()
                .chain(std::iter::once(ident)),
        )
    }
    pub fn parent(&self, names: &impl NameAccess) -> Result<Self, NameError> {
        self.check(names)?;
        if self.is_empty() {
            return Ok(self.clone());
        }
        self.slice(names, 0..self.len() - 1, self.anchor)
    }
    pub fn slice(
        &self,
        names: &impl NameAccess,
        range: Range<usize>,
        anchor: PathAnchor,
    ) -> Result<Self, NameError> {
        self.check(names)?;
        self.check_structural_edit()?;
        let selected = self.segments().get(range).ok_or(TokenError::InvalidRange)?;
        Self::new(names, anchor, selected)
    }
    pub fn join(&self, names: &impl NameAccess, suffix: &AstPath) -> Result<Self, NameError> {
        self.check(names)?;
        suffix.check(names)?;
        self.check_structural_edit()?;
        suffix.check_structural_edit()?;
        if suffix.anchor != PathAnchor::Relative {
            return Err(TokenError::InvalidInput("path join suffix must be relative").into());
        }
        if suffix.is_empty() {
            return Ok(self.clone());
        }
        if self.is_empty() {
            return Ok(suffix.with_anchor(self.anchor));
        }
        let count = self
            .len()
            .checked_add(suffix.len())
            .ok_or(TokenError::IdExhausted)?;
        Self::build(
            names,
            self.anchor,
            count,
            self.segments().iter().chain(suffix.segments()).copied(),
        )
    }
    pub fn with_anchor(&self, anchor: PathAnchor) -> Self {
        Self {
            anchor,
            data: self.data.clone(),
            resolved_use: self.resolved_use.clone(),
        }
    }

    /// Diagnostic-only ownership boundary, never a semantic lookup key.
    /// Construction charges actual capacity as DiagnosticBytes and retains the
    /// backing lease in the returned owner. No bare String escapes its lease.
    pub fn display(&self, names: &impl NameAccess) -> Result<PathDisplay, NameError> {
        self.check(names)?;
        let prefix = match self.anchor {
            PathAnchor::Relative => 0,
            PathAnchor::Crate => 5,
            PathAnchor::SelfModule | PathAnchor::SelfType => 4,
            PathAnchor::Super(0) => 0,
            PathAnchor::Super(count) => usize::from(count)
                .checked_mul(7)
                .and_then(|n| n.checked_sub(2))
                .ok_or(TokenError::IdExhausted)?,
        };
        let mut capacity = prefix;
        for (index, ident) in self.segments().iter().enumerate() {
            let separator = if prefix > 0 || index > 0 { 2 } else { 0 };
            let length = names.spelling(*ident)?.len();
            capacity = capacity
                .checked_add(separator)
                .and_then(|n| n.checked_add(length))
                .ok_or(TokenError::IdExhausted)?;
        }
        let budget = names.budget();
        budget.charge(
            Counter::Work,
            (capacity as u64)
                .checked_add(1)
                .ok_or(TokenError::IdExhausted)?,
        )?;
        budget.charge(Counter::DiagnosticBytes, capacity as u64)?;
        let mut lease = budget.reserve(Storage::Retained, capacity as u64)?;
        let mut output = String::new();
        output
            .try_reserve_exact(capacity)
            .map_err(|_| budget.poison_allocation())?;
        lease.resize(output.capacity() as u64)?;
        budget.charge(
            Counter::DiagnosticBytes,
            output.capacity().saturating_sub(capacity) as u64,
        )?;
        match self.anchor {
            PathAnchor::Relative => {}
            PathAnchor::Crate => output.push_str("crate"),
            PathAnchor::SelfModule => output.push_str("self"),
            PathAnchor::SelfType => output.push_str("Self"),
            PathAnchor::Super(count) => {
                for index in 0..count {
                    if index > 0 {
                        output.push_str("::");
                    }
                    output.push_str("super");
                }
            }
        }
        for ident in self.segments() {
            if !output.is_empty() {
                output.push_str("::");
            }
            output.push_str(names.spelling(*ident)?);
        }
        Ok(PathDisplay {
            text: output,
            _lease: lease,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::expansion_budget::{BudgetResource, ExpansionLimits};
    use super::super::token::Edition;
    use super::super::token_session::SourceSnapshot;
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::path::Path;

    fn fixture(budget: ExpansionBudget) -> (NameStoreBuilder, [AstIdent; 3]) {
        let source =
            SourceSnapshot::from_text(Path::new("path-fixture.rs"), "abc", &budget).unwrap();
        let file = source.file(0).unwrap();
        let first = source.span(file, 0, 1).unwrap();
        let eof = source.span(file, 3, 3).unwrap();
        let (mut session, session_cap) = TokenSession::standalone(source, budget).unwrap();
        let first = session.original_span(first).unwrap();
        let eof = session.original_span(eof).unwrap();
        let mut names = NameStoreBuilder::new(session, session_cap, Edition::E2024).unwrap();
        let a = names.intern_root("alpha", first).unwrap();
        let other_a = names.intern_root("alpha", eof).unwrap();
        let b = names.intern_root("beta", eof).unwrap();
        (names, [a, other_a, b])
    }
    fn hash(key: &PathKey) -> u64 {
        let mut state = DefaultHasher::new();
        key.hash(&mut state);
        state.finish()
    }

    #[test]
    fn resolved_use_keeps_original_occurrences_without_changing_semantic_keys() {
        let (names, [declaration, occurrence, other]) = fixture(ExpansionBudget::default());
        let target = AstPath::new(&names, PathAnchor::Crate, &[declaration]).unwrap();
        let source = AstPath::new(&names, PathAnchor::Relative, &[occurrence]).unwrap();
        let resolved = target.with_resolved_use(&names, &source).unwrap();
        let rebuilt = AstPath::new(&names, PathAnchor::Crate, &[declaration]).unwrap();
        resolved.check_target(&names, &rebuilt).unwrap();
        let same_key_other_occurrence =
            AstPath::new(&names, PathAnchor::Crate, &[occurrence]).unwrap();
        assert_eq!(
            rebuilt.key(Namespace::Type),
            same_key_other_occurrence.key(Namespace::Type)
        );
        assert!(matches!(
            resolved.check_target(&names, &same_key_other_occurrence),
            Err(NameError::Token(TokenError::InvalidInput(_)))
        ));
        assert_eq!(resolved.key(Namespace::Value), target.key(Namespace::Value));
        assert_eq!(
            hash(&resolved.key(Namespace::Value)),
            hash(&target.key(Namespace::Value))
        );
        assert_eq!(
            names
                .span(resolved.source_occurrences(&names).unwrap()[0])
                .unwrap(),
            names.span(occurrence).unwrap()
        );
        assert_ne!(
            names.span(resolved.segments()[0]).unwrap(),
            names.span(occurrence).unwrap()
        );
        let next = AstPath::new(&names, PathAnchor::Crate, &[other]).unwrap();
        let next = next.with_resolved_use(&names, &resolved).unwrap();
        assert!(Arc::ptr_eq(
            &next.resolved_use.as_ref().unwrap().original,
            &source.data
        ));
        let original = next.original_use(&names).unwrap();
        assert_eq!(original.anchor(), PathAnchor::Relative);
        assert!(original.resolved_use.is_none());
        assert!(Arc::ptr_eq(&original.data, &source.data));
    }

    #[test]
    fn resolved_use_cannot_be_rebound_by_public_structural_operations() {
        let (names, [declaration, occurrence, other]) = fixture(ExpansionBudget::default());
        let target = AstPath::new(&names, PathAnchor::Crate, &[declaration]).unwrap();
        let source = AstPath::new(&names, PathAnchor::Relative, &[occurrence]).unwrap();
        let resolved = target.with_resolved_use(&names, &source).unwrap();
        assert!(matches!(
            resolved.push(&names, other),
            Err(NameError::Token(TokenError::InvalidInput(_)))
        ));
        assert!(matches!(
            resolved.slice(&names, 0..1, PathAnchor::Relative),
            Err(NameError::Token(TokenError::InvalidInput(_)))
        ));
        assert!(matches!(
            resolved.with_anchor(PathAnchor::Relative).check(&names),
            Err(NameError::Token(TokenError::InvalidInput(_)))
        ));
        let mut altered = resolved.clone();
        altered.data = AstPath::new(&names, PathAnchor::Crate, &[other])
            .unwrap()
            .data;
        assert!(matches!(
            altered.check(&names),
            Err(NameError::Token(TokenError::InvalidInput(_)))
        ));
        let (foreign, [foreign_ident, _, _]) = fixture(ExpansionBudget::default());
        let foreign_path = AstPath::new(&foreign, PathAnchor::Relative, &[foreign_ident]).unwrap();
        assert!(matches!(
            target.with_resolved_use(&names, &foreign_path),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
    }

    #[test]
    fn resolved_use_certificate_shares_storage_and_releases_on_last_owner() {
        let budget = ExpansionBudget::default();
        let (names, [declaration, occurrence, _]) = fixture(budget.clone());
        let target = AstPath::new(&names, PathAnchor::Crate, &[declaration]).unwrap();
        let source = AstPath::new(&names, PathAnchor::Relative, &[occurrence]).unwrap();
        let before = budget.snapshot().retained_bytes;
        let resolved = target.with_resolved_use(&names, &source).unwrap();
        assert_eq!(
            budget.snapshot().retained_bytes - before,
            (size_of::<ResolvedUse>() + 2 * size_of::<usize>()) as u64
        );
        let clone = resolved.clone();
        let retained = budget.snapshot().retained_bytes;
        assert!(Arc::ptr_eq(
            resolved.resolved_use.as_ref().unwrap(),
            clone.resolved_use.as_ref().unwrap()
        ));
        drop(resolved);
        assert_eq!(budget.snapshot().retained_bytes, retained);
        drop(source);
        drop(target);
        drop(names);
        assert!(budget.snapshot().retained_bytes > 0);
        drop(clone);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn resolved_use_denies_storage_before_certificate_allocation() {
        let healthy = ExpansionBudget::default();
        let (names, [declaration, occurrence, _]) = fixture(healthy.clone());
        let target = AstPath::new(&names, PathAnchor::Crate, &[declaration]).unwrap();
        let source = AstPath::new(&names, PathAnchor::Relative, &[occurrence]).unwrap();
        let baseline = healthy.snapshot().retained_bytes;
        drop((target, source, names));
        let fixed = (size_of::<ResolvedUse>() + 2 * size_of::<usize>()) as u64;
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes: baseline + fixed - 1,
            ..ExpansionLimits::default()
        });
        let (names, [declaration, occurrence, _]) = fixture(budget.clone());
        let target = AstPath::new(&names, PathAnchor::Crate, &[declaration]).unwrap();
        let source = AstPath::new(&names, PathAnchor::Relative, &[occurrence]).unwrap();
        let key = target.key(Namespace::Value);
        let before_hash = hash(&key);
        let Err(NameError::Token(TokenError::Budget(first))) =
            target.with_resolved_use(&names, &source)
        else {
            panic!("certificate allocation must be admitted first")
        };
        assert_eq!(budget.snapshot().retained_bytes, baseline);
        let Err(NameError::Token(TokenError::Budget(again))) =
            target.with_resolved_use(&names, &source)
        else {
            panic!("certificate admission failure must latch")
        };
        assert_eq!(first, again);
        assert_eq!(hash(&key), before_hash);
        drop((target, source, names, key));
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn semantic_keys_ignore_occurrence_spans_but_keep_anchor_namespace_and_owner() {
        let (names, [a, other_a, b]) = fixture(ExpansionBudget::default());
        assert_ne!(names.span(a).unwrap(), names.span(other_a).unwrap());
        let path = AstPath::new(&names, PathAnchor::Relative, &[a, b]).unwrap();
        let other = AstPath::new(&names, PathAnchor::Relative, &[other_a, b]).unwrap();
        assert_eq!(path.key(Namespace::Value), other.key(Namespace::Value));
        assert_eq!(
            hash(&path.key(Namespace::Value)),
            hash(&other.key(Namespace::Value))
        );
        assert_ne!(path.key(Namespace::Value), path.key(Namespace::Type));
        assert_ne!(
            path.key(Namespace::Value),
            path.with_anchor(PathAnchor::Crate).key(Namespace::Value)
        );
        let (foreign, [foreign_a, _, foreign_b]) = fixture(ExpansionBudget::default());
        let foreign_path =
            AstPath::new(&foreign, PathAnchor::Relative, &[foreign_a, foreign_b]).unwrap();
        assert_ne!(
            path.key(Namespace::Value),
            foreign_path.key(Namespace::Value)
        );
    }

    #[test]
    fn empty_paths_retain_owners_and_cannot_join_foreign_roots() {
        let (names, [a, _, _]) = fixture(ExpansionBudget::default());
        let (foreign, _) = fixture(ExpansionBudget::default());
        let root = AstPath::new(&names, PathAnchor::Crate, &[]).unwrap();
        let other_root = AstPath::new(&foreign, PathAnchor::Crate, &[]).unwrap();
        assert!(root.is_empty());
        assert_ne!(root.key(Namespace::Type), other_root.key(Namespace::Type));
        assert!(matches!(
            root.check(&foreign),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            root.push(&foreign, a),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        let foreign_empty = other_root.with_anchor(PathAnchor::Relative);
        assert!(matches!(
            root.join(&names, &foreign_empty),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert!(matches!(
            AstPath::new(&foreign, PathAnchor::Relative, &[a]),
            Err(NameError::Token(TokenError::ForeignHandle))
        ));
        assert_eq!(
            root.parent(&names).unwrap().key(Namespace::Type),
            root.key(Namespace::Type)
        );
    }

    #[test]
    fn typed_path_operations_and_diagnostic_anchors_preserve_semantics() {
        let (names, [a, _, b]) = fixture(ExpansionBudget::default());
        let first = AstPath::new(&names, PathAnchor::Relative, &[a]).unwrap();
        let selected = first.single().unwrap();
        assert_eq!(
            names.key(selected, LookupClass::Local).unwrap(),
            names.key(a, LookupClass::Local).unwrap()
        );
        let path = first.push(&names, b).unwrap();
        assert_eq!(path.len(), 2);
        assert!(path.single().is_none());
        let suffix = path.slice(&names, 1..2, PathAnchor::Relative).unwrap();
        assert_eq!(
            first.join(&names, &suffix).unwrap().key(Namespace::Type),
            path.key(Namespace::Type)
        );
        assert_eq!(
            path.parent(&names).unwrap().key(Namespace::Type),
            first.key(Namespace::Type)
        );
        assert!(matches!(
            path.slice(&names, 1..3, PathAnchor::Relative),
            Err(NameError::Token(TokenError::InvalidRange))
        ));
        let rooted = first.with_anchor(PathAnchor::Crate);
        assert!(rooted.single().is_none());
        assert!(matches!(
            first.join(&names, &rooted),
            Err(NameError::Token(TokenError::InvalidInput(_)))
        ));
        for (anchor, expected) in [
            (PathAnchor::Relative, "alpha::beta"),
            (PathAnchor::Crate, "crate::alpha::beta"),
            (PathAnchor::SelfModule, "self::alpha::beta"),
            (PathAnchor::Super(2), "super::super::alpha::beta"),
            (PathAnchor::SelfType, "Self::alpha::beta"),
        ] {
            let selected = path.with_anchor(anchor);
            assert_eq!(selected.anchor(), anchor);
            assert_eq!(selected.display(&names).unwrap().as_str(), expected);
        }
        let frozen = names.freeze().unwrap();
        path.check(frozen.as_ref()).unwrap();
        assert_eq!(
            path.display(frozen.as_ref()).unwrap().as_str(),
            "alpha::beta"
        );
    }

    #[test]
    fn canonical_paths_can_exceed_the_explicit_source_path_fence() {
        let (names, [a, _, b]) = fixture(ExpansionBudget::default());
        let segments = vec![a; 129];
        let path = AstPath::new(&names, PathAnchor::Crate, &segments).unwrap();
        let member = path.push(&names, b).unwrap();
        assert_eq!(member.len(), 130);
        assert_eq!(
            member.parent(&names).unwrap().key(Namespace::Type),
            path.key(Namespace::Type)
        );
    }

    #[test]
    fn allocation_free_keys_and_anchor_clones_hold_the_last_path_data_lease() {
        let budget = ExpansionBudget::default();
        let (names, [a, _, b]) = fixture(budget.clone());
        let path = AstPath::new(&names, PathAnchor::Relative, &[a, b]).unwrap();
        let before = budget.snapshot();
        let key = path.key(Namespace::Value);
        let anchor_clone = path.with_anchor(PathAnchor::Crate);
        assert!(Arc::ptr_eq(&path.data, &key.data));
        assert!(Arc::ptr_eq(&path.data, &anchor_clone.data));
        assert_eq!(budget.snapshot().retained_bytes, before.retained_bytes);
        assert_eq!(budget.snapshot().counters, before.counters);
        drop(path);
        drop(anchor_clone);
        drop(names);
        assert!(budget.snapshot().retained_bytes > 0);
        assert_eq!(key, key.clone());
        drop(key);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn display_retains_actual_backing_after_path_and_name_owners_drop() {
        let budget = ExpansionBudget::default();
        let (names, [a, _, b]) = fixture(budget.clone());
        let path = AstPath::new(&names, PathAnchor::Crate, &[a, b]).unwrap();
        let before = budget.snapshot().counters[Counter::DiagnosticBytes as usize];
        let display = path.display(&names).unwrap();
        let capacity = display.text.capacity() as u64;
        assert_eq!(
            budget.snapshot().counters[Counter::DiagnosticBytes as usize] - before,
            capacity
        );
        drop(path);
        drop(names);
        assert_eq!(budget.snapshot().retained_bytes, capacity);
        assert_eq!(display.as_str(), "crate::alpha::beta");
        drop(display);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn partial_path_construction_releases_backing_and_preserves_failure() {
        let healthy = ExpansionBudget::default();
        let (names, _) = fixture(healthy.clone());
        let baseline = healthy.snapshot().retained_bytes;
        drop(names);
        let fixed = (size_of::<PathData>() + 2 * size_of::<usize>()) as u64;
        // Admit the fixed owner and first occurrence vector, but not its key vector.
        let bytes = baseline + fixed + (4 * size_of::<AstIdent>()) as u64 + 1;
        let budget = ExpansionBudget::new(ExpansionLimits {
            bytes,
            ..ExpansionLimits::default()
        });
        let (names, [a, _, _]) = fixture(budget.clone());
        let Err(NameError::Token(TokenError::Budget(first))) =
            AstPath::new(&names, PathAnchor::Crate, &[a])
        else {
            panic!("key capacity must be admitted")
        };
        assert_eq!(budget.snapshot().retained_bytes, baseline);
        let Err(NameError::Token(TokenError::Budget(again))) =
            AstPath::new(&names, PathAnchor::Crate, &[a])
        else {
            panic!("failure must latch")
        };
        assert_eq!(first, again);
        drop(names);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }

    #[test]
    fn diagnostic_failure_is_typed_and_cannot_corrupt_eq_or_hash() {
        let mut counters = ExpansionLimits::default().counters;
        counters[Counter::DiagnosticBytes as usize] = 0;
        let budget = ExpansionBudget::new(ExpansionLimits {
            counters,
            ..ExpansionLimits::default()
        });
        let (names, [a, _, _]) = fixture(budget.clone());
        let path = AstPath::new(&names, PathAnchor::Relative, &[a]).unwrap();
        let key = path.key(Namespace::Value);
        let before_hash = hash(&key);
        let retained = budget.snapshot().retained_bytes;
        let Err(NameError::Token(TokenError::Budget(first))) = path.display(&names) else {
            panic!("diagnostic bytes must be admitted")
        };
        assert_eq!(
            first.resource,
            BudgetResource::Counter(Counter::DiagnosticBytes)
        );
        assert_eq!(budget.snapshot().retained_bytes, retained);
        assert_eq!(key, key.clone());
        assert_eq!(hash(&key), before_hash);
        let Err(NameError::Token(TokenError::Budget(again))) = path.check(&names) else {
            panic!("failure must latch")
        };
        assert_eq!(first, again);
        drop(key);
        drop(path);
        drop(names);
        assert_eq!(budget.snapshot().retained_bytes, 0);
    }
}
