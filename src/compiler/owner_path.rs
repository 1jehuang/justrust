//! Structural owner identities (PARSE-SPLIT.md, "Increment 1 owner derivation").
//!
//! An [`OwnerPath`] names who issued a handle: the producer's [`OwnerRoot`]
//! followed by `(domain, ordinal)` steps.
//!
//! Distinct issuances never share a path because:
//! - An `OwnerRoot` is minted once per graph node execution
//!   (`Context::owner_root` rejects a second mint), or under a fresh engine id
//!   for standalone work. It is neither `Copy` nor `Clone`.
//! - [`OwnerIssuer::new`] consumes the root, so one root yields exactly one
//!   top-level issuer and one ordinal sequence.
//! - Every child, files included, comes from [`OwnerIssuer::issue`] on an
//!   exclusively borrowed parent. Each child gets its own issuer, so the only
//!   way to obtain an issuer at a given path is to be the one issuance that
//!   created it. There is no positional or repeatable factory.
//!
//! Every path step is charged against the caller's [`ExpansionBudget`] before
//! the step storage is allocated, then sealed into an immutable
//! [`RetainedBytes`] receipt. Paths hold no budget handle or lock.
//!
//! Not yet activated: the process-global counters still issue live owner ids.
#![allow(dead_code)]

use super::expansion_budget::{
    BudgetFailure, ByteLease, Counter, ExpansionBudget, RetainedBytes, Storage,
};
use crate::{EngineAuthority, OwnerRoot, OwnerTag, TagMint};
use std::sync::Arc;

/// Closed set of owner domains. A step's domain is part of its identity, so a
/// session and a module arena at the same position never compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OwnerDomain {
    /// One source file unit, issued by its parent in declaration order.
    File,
    SourceSnapshot,
    TokenSession,
    ModuleArena,
    ScopeArena,
    ItemMacroDriver,
    LocalMacroDriver,
    TokenSlice,
    PreparedView,
}

const DOMAINS: usize = 9;

fn domain_index(domain: OwnerDomain) -> usize {
    match domain {
        OwnerDomain::File => 0,
        OwnerDomain::SourceSnapshot => 1,
        OwnerDomain::TokenSession => 2,
        OwnerDomain::ModuleArena => 3,
        OwnerDomain::ScopeArena => 4,
        OwnerDomain::ItemMacroDriver => 5,
        OwnerDomain::LocalMacroDriver => 6,
        OwnerDomain::TokenSlice => 7,
        OwnerDomain::PreparedView => 8,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Step {
    domain: OwnerDomain,
    ordinal: u64,
}

/// Maximum path depth, checked before any allocation.
pub const MAX_OWNER_DEPTH: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OwnerError {
    DepthExceeded,
    OrdinalExhausted,
    TagExhausted,
    Budget(BudgetFailure),
    Allocation,
}

impl From<BudgetFailure> for OwnerError {
    fn from(failure: BudgetFailure) -> Self {
        Self::Budget(failure)
    }
}

/// The producer's identity, shared by every path it issues. Fixed size: an
/// engine authority and an engine-local node id, never a copied key payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct RootId {
    authority: EngineAuthority,
    producer: Option<u64>,
}

/// A complete, immutable, published owner identity. Its storage is paid by
/// one [`RetainedBytes`] receipt: no lease, budget handle or lock is reachable
/// from it. Created only by [`BuilderOwnedPath::freeze`]. Clones share the
/// storage. Equality and hashing compare the full structure.
#[derive(Clone, Debug)]
pub struct OwnerPath {
    inner: Arc<PublishedPath>,
}

#[derive(Debug)]
struct PublishedPath {
    root: RootId,
    steps: Box<[Step]>,
    _receipt: RetainedBytes,
}

/// An owner identity still under construction. Its storage is a refundable
/// [`ByteLease`]: dropping it before publication returns the bytes. Consumed
/// exactly once by [`Self::freeze`], which converts the same reservation into
/// an immutable receipt (no second charge, no refund afterwards). Never
/// reachable from a published fact.
#[derive(Debug)]
pub struct BuilderOwnedPath {
    root: RootId,
    steps: Box<[Step]>,
    lease: ByteLease,
    /// The admission ledger that paid for this path. Compared by identity
    /// (not by equal limits) so an owner paid under one budget cannot back a
    /// session or store charged to another.
    payer: ExpansionBudget,
}

impl PartialEq for OwnerPath {
    fn eq(&self, other: &Self) -> bool {
        self.inner.root == other.inner.root && self.inner.steps == other.inner.steps
    }
}
impl Eq for OwnerPath {}
impl std::hash::Hash for OwnerPath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.inner.root.hash(state);
        self.inner.steps.hash(state);
    }
}

impl PartialEq for BuilderOwnedPath {
    fn eq(&self, other: &Self) -> bool {
        self.same_identity(other)
    }
}
impl Eq for BuilderOwnedPath {}

fn descends(root: &RootId, mine: &[Step], ancestor_root: &RootId, theirs: &[Step]) -> bool {
    root == ancestor_root && mine.len() >= theirs.len() && mine[..theirs.len()] == theirs[..]
}

impl OwnerPath {
    pub fn authority(&self) -> EngineAuthority {
        self.inner.root.authority
    }
    pub fn depth(&self) -> usize {
        self.inner.steps.len()
    }
    /// Whether `self` was issued (transitively) under `ancestor`.
    pub fn descends_from(&self, ancestor: &OwnerPath) -> bool {
        descends(
            &self.inner.root,
            &self.inner.steps,
            &ancestor.inner.root,
            &ancestor.inner.steps,
        )
    }
    pub fn domain(&self) -> Option<OwnerDomain> {
        self.inner.steps.last().map(|step| step.domain)
    }
}

impl BuilderOwnedPath {
    pub fn authority(&self) -> EngineAuthority {
        self.root.authority
    }
    pub fn depth(&self) -> usize {
        self.steps.len()
    }
    pub fn domain(&self) -> Option<OwnerDomain> {
        self.steps.last().map(|step| step.domain)
    }
    /// Whether `self` was issued (transitively) under `ancestor`.
    pub fn descends_from(&self, ancestor: &BuilderOwnedPath) -> bool {
        descends(&self.root, &self.steps, &ancestor.root, &ancestor.steps)
    }
    /// Whether `budget` is the exact ledger that paid for this path.
    pub fn paid_by(&self, budget: &ExpansionBudget) -> bool {
        self.payer.same_authority(budget)
    }
    /// Structural equality with a builder path (not identity of storage).
    pub fn same_identity(&self, other: &BuilderOwnedPath) -> bool {
        self.root == other.root && self.steps == other.steps
    }
    /// Publish: convert the refundable reservation into an immutable receipt
    /// for the same bytes, exactly once. Consumes the builder path, so it can
    /// be neither refunded nor charged again.
    pub fn freeze(self) -> Result<OwnerPath, OwnerError> {
        let receipt = self.lease.retain()?;
        Ok(OwnerPath {
            inner: Arc::new(PublishedPath {
                root: self.root,
                steps: self.steps,
                _receipt: receipt,
            }),
        })
    }
}

/// Issues child owners from one exclusively held position in the tree.
#[derive(Debug)]
pub struct OwnerIssuer {
    path: BuilderOwnedPath,
    next: [u64; DOMAINS],
    budget: ExpansionBudget,
    mint: TagMint,
}

/// A newly issued owner: its child issuer plus the tag bound to that exact
/// path. The pair is created once in `issue` and cannot be reassembled.
///
/// `Issued` is neither `Copy` nor `Clone`, so one issuance cannot build two
/// owners (regression for the bare-tag replay found in review):
///
/// ```compile_fail
/// fn replay(issued: justrust_engine::compiler::owner_path::Issued) {
///     let again = issued;
///     let _twice = (issued, again);
/// }
/// ```
///
/// The fields are private and there is no public constructor, so a tag from
/// one issuance cannot be paired with another issuer:
///
/// ```compile_fail
/// fn recombine(
///     a: justrust_engine::compiler::owner_path::Issued,
///     b: justrust_engine::compiler::owner_path::Issued,
/// ) {
///     let _ = justrust_engine::compiler::owner_path::Issued { issuer: b.issuer, tag: a.tag };
/// }
/// ```
#[derive(Debug)]
pub struct Issued {
    issuer: OwnerIssuer,
    tag: OwnerTag,
}

impl Issued {
    /// The issued owner's tag, for comparison only. It cannot be used to
    /// construct anything: no public API accepts a bare tag.
    pub fn tag(&self) -> OwnerTag {
        self.tag
    }
    /// The issued owner's builder path, for structural checks.
    pub fn path(&self) -> &BuilderOwnedPath {
        &self.issuer.path
    }
    /// Consume the pair for exactly one owner. Crate-private: only the owner
    /// constructors (`TokenSession::with_owner`, arena/driver builders) split
    /// it, and none of them reassembles a pair.
    pub(crate) fn into_parts(self) -> (OwnerIssuer, OwnerTag) {
        (self.issuer, self.tag)
    }
    /// The issuer, to issue children. Crate-private for the same reason.
    pub(crate) fn into_issuer(self) -> OwnerIssuer {
        self.issuer
    }
}

/// Bytes one owner path of `depth` steps occupies once published, including
/// the shared `Arc` header. The same bytes are reserved while building.
fn path_bytes(depth: usize) -> Result<u64, OwnerError> {
    let bytes = depth
        .checked_mul(std::mem::size_of::<Step>())
        .and_then(|n| {
            n.checked_add(std::mem::size_of::<PublishedPath>() + 2 * std::mem::size_of::<usize>())
        })
        .ok_or(OwnerError::Allocation)?;
    u64::try_from(bytes).map_err(|_| OwnerError::Allocation)
}

/// One paid, refundable builder path: work and bytes are charged before the
/// step storage is allocated.
fn paid_path(
    budget: &ExpansionBudget,
    root: RootId,
    prefix: &[Step],
    last: Option<Step>,
) -> Result<BuilderOwnedPath, OwnerError> {
    let depth = prefix.len() + usize::from(last.is_some());
    if depth > MAX_OWNER_DEPTH {
        return Err(OwnerError::DepthExceeded);
    }
    budget.charge(Counter::Work, depth as u64 + 1)?;
    let lease = budget.reserve(Storage::Retained, path_bytes(depth)?)?;
    let mut steps = Vec::new();
    steps
        .try_reserve_exact(depth)
        .map_err(|_| OwnerError::Allocation)?;
    steps.extend_from_slice(prefix);
    steps.extend(last);
    Ok(BuilderOwnedPath {
        root,
        steps: steps.into_boxed_slice(),
        lease,
        payer: budget.clone(),
    })
}

impl OwnerIssuer {
    /// The top-level issuer for one producer. Consumes the root, which is
    /// minted once per node (`Context::owner_root`) or once per standalone
    /// computation (`OwnerRoot::standalone`).
    pub fn new(root: OwnerRoot, budget: &ExpansionBudget) -> Result<Self, OwnerError> {
        let (authority, producer, mint) = root.into_parts();
        Ok(Self {
            path: paid_path(
                budget,
                RootId {
                    authority,
                    producer,
                },
                &[],
                None,
            )?,
            next: [0; DOMAINS],
            budget: budget.clone(),
            mint,
        })
    }

    pub fn path(&self) -> &BuilderOwnedPath {
        &self.path
    }

    /// Issue the next child of `domain` with its bound tag. The only way to
    /// create a child or a tag. All checks and payments happen before the
    /// ordinal or tag is consumed, so a failure reuses nothing.
    pub fn issue(&mut self, domain: OwnerDomain) -> Result<Issued, OwnerError> {
        let slot = &mut self.next[domain_index(domain)];
        let ordinal = *slot;
        let following = ordinal.checked_add(1).ok_or(OwnerError::OrdinalExhausted)?;
        // The tag is stored in the caller's already-paid container: work only.
        self.budget.charge(Counter::Work, 1)?;
        let path = paid_path(
            &self.budget,
            self.path.root,
            &self.path.steps,
            Some(Step { domain, ordinal }),
        )?;
        let tag = self.mint.mint().map_err(|_| OwnerError::TagExhausted)?;
        // Commit the ordinal only after the child and tag exist.
        *slot = following;
        Ok(Issued {
            issuer: OwnerIssuer {
                path,
                next: [0; DOMAINS],
                budget: self.budget.clone(),
                mint: self.mint.derive(),
            },
            tag,
        })
    }

    /// The builder path without the issuing capability. Dropping the issuer
    /// drops its budget handle and mint; the path keeps only its own lease.
    pub fn into_path(self) -> BuilderOwnedPath {
        self.path
    }
}

/// Peak bytes a standalone session owner reserves while it is issued: the
/// root issuer's path plus the session's child path. Admitted up front.
pub fn standalone_admission_bytes() -> u64 {
    path_bytes(0).unwrap_or(0) + path_bytes(1).unwrap_or(0)
}

/// Bytes that stay charged after a standalone session's store is frozen:
/// only the session's own published path receipt. The issuer's root path is
/// refunded when the issuer drops; the receipt is never refunded.
pub fn standalone_session_receipt_bytes() -> u64 {
    path_bytes(1).unwrap_or(0)
}

/// One session owner under a fresh standalone root, for independent direct
/// tooling (`TokenSession::standalone`). Graph code never calls this; it
/// receives an `Issued` from its producer's issuer. Everything is paid by the
/// caller's budget, and admission is checked before any root is minted.
pub(super) fn standalone_issued(
    budget: &ExpansionBudget,
) -> Result<Issued, super::token_session::TokenError> {
    // Admit the whole owner cost first, so an exhausted or zero caller budget
    // rejects before an engine id is consumed or anything is allocated. The
    // probe is released immediately; the real charges follow below.
    budget.check()?;
    drop(budget.reserve(Storage::Retained, standalone_admission_bytes())?);
    let root =
        OwnerRoot::standalone().map_err(|_| super::token_session::TokenError::IdExhausted)?;
    let mut issuer = OwnerIssuer::new(root, budget).map_err(owner_token_error)?;
    issuer
        .issue(OwnerDomain::TokenSession)
        .map_err(owner_token_error)
}

fn owner_token_error(error: OwnerError) -> super::token_session::TokenError {
    match error {
        OwnerError::Budget(failure) => failure.into(),
        _ => super::token_session::TokenError::IdExhausted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test helper: issue a child and keep only its issuer.
    trait Child {
        fn child(&mut self, domain: OwnerDomain) -> Result<OwnerIssuer, OwnerError>;
    }
    impl Child for OwnerIssuer {
        fn child(&mut self, domain: OwnerDomain) -> Result<OwnerIssuer, OwnerError> {
            self.issue(domain).map(|issued| issued.into_issuer())
        }
    }

    fn budget() -> ExpansionBudget {
        ExpansionBudget::default()
    }

    fn standalone() -> OwnerIssuer {
        OwnerIssuer::new(OwnerRoot::standalone().unwrap(), &budget()).unwrap()
    }

    #[test]
    fn siblings_are_distinct_and_domains_are_separated() {
        let mut root = standalone();
        let a = root.child(OwnerDomain::TokenSession).unwrap();
        let b = root.child(OwnerDomain::TokenSession).unwrap();
        let m = root.child(OwnerDomain::ModuleArena).unwrap();
        assert_ne!(a.path(), b.path());
        assert_ne!(a.path(), m.path());
        assert_eq!(a.path().domain(), Some(OwnerDomain::TokenSession));
    }

    #[test]
    fn repeated_standalone_roots_never_collide() {
        // Adversarial: build the same structure twice from fresh roots.
        let mut first = standalone();
        let mut second = standalone();
        let x = first
            .child(OwnerDomain::File)
            .unwrap()
            .child(OwnerDomain::TokenSession)
            .unwrap();
        let y = second
            .child(OwnerDomain::File)
            .unwrap()
            .child(OwnerDomain::TokenSession)
            .unwrap();
        assert_ne!(first.path(), second.path());
        assert_ne!(x.path(), y.path());
    }

    #[test]
    fn files_are_issued_not_positional() {
        // Two files of one parent are distinct; re-issuing never repeats a file.
        let mut root = standalone();
        let mut file0 = root.child(OwnerDomain::File).unwrap();
        let mut file1 = root.child(OwnerDomain::File).unwrap();
        let s0 = file0.child(OwnerDomain::TokenSession).unwrap();
        let s1 = file1.child(OwnerDomain::TokenSession).unwrap();
        assert_ne!(file0.path(), file1.path());
        assert_ne!(s0.path(), s1.path());
        assert!(s0.path().descends_from(file0.path()));
        assert!(!s0.path().descends_from(file1.path()));
        assert!(s0.path().descends_from(root.path()));
    }

    #[test]
    fn concurrent_standalone_roots_are_distinct() {
        let paths: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| scope.spawn(|| standalone().into_path()))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        for (i, a) in paths.iter().enumerate() {
            for b in &paths[i + 1..] {
                assert_ne!(a, b);
            }
        }
    }

    #[test]
    fn depth_is_bounded_before_allocation() {
        let mut issuer = standalone();
        for _ in 0..MAX_OWNER_DEPTH {
            issuer = issuer.child(OwnerDomain::TokenSlice).unwrap();
        }
        assert_eq!(
            issuer.child(OwnerDomain::TokenSlice).unwrap_err(),
            OwnerError::DepthExceeded
        );
    }

    #[test]
    fn issue_is_paid_before_allocation_and_failure_is_sticky() {
        use super::super::expansion_budget::ExpansionLimits;
        // Room for the root path plus exactly one depth-1 child.
        let small = ExpansionBudget::new(ExpansionLimits {
            bytes: path_bytes(0).unwrap() + path_bytes(1).unwrap(),
            ..ExpansionLimits::default()
        });
        let mut root = OwnerIssuer::new(OwnerRoot::standalone().unwrap(), &small).unwrap();
        let before = small.snapshot();
        let _first = root.child(OwnerDomain::TokenSession).unwrap();
        assert!(
            small.snapshot().retained_bytes > before.retained_bytes,
            "issue reserved retained bytes"
        );
        // The next child does not fit: rejected by admission, not by an
        // allocation that already happened.
        assert!(matches!(
            root.child(OwnerDomain::TokenSession),
            Err(OwnerError::Budget(_))
        ));
        // Expansion budgets latch the first failure: no later issue succeeds.
        assert!(root.child(OwnerDomain::ModuleArena).is_err());
    }

    #[test]
    fn engine_roots_are_single_mint_and_distinct_across_engines() {
        use crate::{Context, Engine, Key, Provider, TaskFuture};
        use std::sync::Mutex;
        type Seen = Arc<Mutex<Vec<(EngineAuthority, bool)>>>;
        struct Mint(Seen);
        impl Provider for Mint {
            fn outputs(&self, _: &Key) -> usize {
                1
            }
            fn start(&self, cx: Context, _: Key) -> TaskFuture {
                let seen = self.0.clone();
                Box::pin(async move {
                    let first = cx.owner_root().expect("first mint succeeds");
                    let second = cx.owner_root().is_err();
                    seen.lock().unwrap().push((first.authority(), second));
                    cx.publish(0, 1)
                })
            }
        }
        let seen: Seen = Arc::default();
        let mut authorities = Vec::new();
        for _ in 0..2 {
            let engine = Engine::new(Mint(seen.clone()));
            authorities.push(engine.authority());
            let demand = engine.request(Key::new(1, vec![]));
            engine.run(1).unwrap();
            demand.completion().unwrap();
        }
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen.iter().all(|(_, second_rejected)| *second_rejected));
        assert_ne!(seen[0].0, seen[1].0);
        assert_eq!(seen[0].0, authorities[0]);
    }

    #[test]
    fn owner_root_lifecycle_across_pending_failure_and_reexecution() {
        use crate::{Context, Engine, Key, Provider, TaskFuture};
        use std::sync::Mutex;
        // Key 1: mints, suspends on key 2 (Pending), then issues a child from
        // the root it still owns locally. Key 2: mints and fails after minting.
        type Log = Arc<Mutex<Vec<String>>>;
        struct Life(Log, bool);
        impl Provider for Life {
            fn outputs(&self, _: &Key) -> usize {
                1
            }
            fn start(&self, cx: Context, key: Key) -> TaskFuture {
                let log = self.0.clone();
                let fail = self.1;
                Box::pin(async move {
                    let root = cx.owner_root()?;
                    if key.kind == 2 {
                        log.lock()
                            .unwrap()
                            .push(format!("minted 2 {}", root.authority().get()));
                        if fail {
                            return Err(crate::Error::new("fails after minting"));
                        }
                        return cx.publish(0, 7);
                    }
                    let mut issuer =
                        OwnerIssuer::new(root, &ExpansionBudget::default()).expect("issuer");
                    // Suspends: the root is moved into the future and owned
                    // across polls; no re-mint happens on resume.
                    let value = cx.read(cx.request(Key::new(2, vec![])).get(0)).await;
                    let child = issuer.child(OwnerDomain::TokenSession).expect("child");
                    assert!(child.path().descends_from(issuer.path()));
                    log.lock()
                        .unwrap()
                        .push(format!("resumed 1 ok={}", value.is_ok()));
                    match value {
                        Ok(_) => cx.publish(0, 1),
                        Err(error) => Err(error),
                    }
                })
            }
        }
        let run = |fail: bool| {
            let log: Log = Arc::default();
            let engine = Engine::new(Life(log.clone(), fail));
            let authority = engine.authority();
            let demand = engine.request(Key::new(1, vec![]));
            engine.run(2).unwrap();
            let outcome = demand.completion();
            let log = log.lock().unwrap().clone();
            (authority, outcome, log)
        };
        // Success: one mint per node, the suspended node resumes with its root.
        let (a, outcome, log) = run(false);
        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(log.len(), 2, "{log:?}");
        assert!(log[0].starts_with("minted 2 "));
        assert_eq!(log[1], "resumed 1 ok=true");
        // Failure after minting: the dependent observes the error and the
        // mint does not leak into later engines.
        let (b, outcome, log) = run(true);
        assert!(outcome.is_err());
        assert_eq!(log.last().map(String::as_str), Some("resumed 1 ok=false"));
        // Deterministic re-execution in a fresh engine mints again under a
        // new authority; no cross-engine mint state exists.
        let (c, outcome, _) = run(false);
        assert!(outcome.is_ok());
        assert!(a != b && b != c && a != c);
    }

    #[test]
    fn builder_path_refunds_until_frozen_then_receipt_is_permanent() {
        let budget = budget();
        let before = budget.snapshot().retained_bytes;
        // Unfrozen: dropping refunds everything.
        let mut root = OwnerIssuer::new(OwnerRoot::standalone().unwrap(), &budget).unwrap();
        let child = root.child(OwnerDomain::TokenSession).unwrap().into_path();
        assert!(
            budget.snapshot().retained_bytes
                >= before + path_bytes(0).unwrap() + path_bytes(1).unwrap()
        );
        drop(child);
        drop(root);
        assert_eq!(
            budget.snapshot().retained_bytes,
            before,
            "unfrozen owner leaked"
        );
        // Frozen: the same reservation becomes a receipt, charged exactly once.
        let mut root = OwnerIssuer::new(OwnerRoot::standalone().unwrap(), &budget).unwrap();
        let child = root.child(OwnerDomain::TokenSession).unwrap().into_path();
        let reserved = budget.snapshot().retained_bytes;
        let published = child.freeze().unwrap();
        assert_eq!(
            budget.snapshot().retained_bytes,
            reserved,
            "freeze charged twice"
        );
        drop(root);
        let after_issuer = budget.snapshot().retained_bytes;
        assert_eq!(after_issuer, before + path_bytes(1).unwrap());
        // Receipts never refund, even after the last clone and the budget drop.
        let copy = published.clone();
        drop(published);
        assert_eq!(budget.snapshot().retained_bytes, after_issuer);
        drop(budget);
        assert_eq!(copy.depth(), 1);
        fn assert_send_sync<T: Send + Sync>(_: &T) {}
        assert_send_sync(&copy);
    }

    #[test]
    fn tags_are_distinct_across_siblings_roots_and_threads() {
        // Siblings of one parent, and identical structure under two roots.
        let mut a = standalone();
        let mut b = standalone();
        let a0 = a.issue(OwnerDomain::TokenSession).unwrap();
        let a1 = a.issue(OwnerDomain::TokenSession).unwrap();
        let b0 = b.issue(OwnerDomain::TokenSession).unwrap();
        assert_ne!(a0.tag(), a1.tag(), "sibling sessions share a tag");
        assert_ne!(
            a0.tag(),
            b0.tag(),
            "identical structure under two roots shares a tag"
        );
        // The path is the authority; equal structure under distinct roots
        // still differs, and tags never stand in for paths.
        assert_ne!(a0.path(), b0.path());
        // Concurrent issuance from independent roots on many threads.
        let tags: Vec<OwnerTag> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| {
                    scope.spawn(|| {
                        let mut root = standalone();
                        (0..8)
                            .map(|_| root.issue(OwnerDomain::TokenSlice).unwrap().tag())
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap())
                .collect()
        });
        let unique: std::collections::HashSet<_> = tags.iter().collect();
        assert_eq!(unique.len(), tags.len(), "concurrent tags collided");
    }

    #[test]
    fn engine_and_standalone_tags_never_collide() {
        use crate::{Context, Engine, Key, Provider, TaskFuture};
        use std::sync::Mutex;
        type Seen = Arc<Mutex<Vec<OwnerTag>>>;
        struct Issue(Seen);
        impl Provider for Issue {
            fn outputs(&self, _: &Key) -> usize {
                1
            }
            fn start(&self, cx: Context, _: Key) -> TaskFuture {
                let seen = self.0.clone();
                Box::pin(async move {
                    let mut issuer =
                        OwnerIssuer::new(cx.owner_root()?, &ExpansionBudget::default())
                            .map_err(|e| crate::Error::new(format!("{e:?}")))?;
                    let tag = issuer.issue(OwnerDomain::TokenSession).unwrap().tag();
                    seen.lock().unwrap().push(tag);
                    cx.publish(0, 1)
                })
            }
        }
        let seen: Seen = Arc::default();
        for _ in 0..3 {
            let engine = Engine::new(Issue(seen.clone()));
            let demand = engine.request(Key::new(1, vec![]));
            engine.run(2).unwrap();
            demand.completion().unwrap();
        }
        let mut tags = seen.lock().unwrap().clone();
        tags.push(standalone().issue(OwnerDomain::TokenSession).unwrap().tag());
        let unique: std::collections::HashSet<_> = tags.iter().collect();
        assert_eq!(unique.len(), 4, "engine/standalone tags collided: {tags:?}");
    }

    #[test]
    fn failed_issue_consumes_no_ordinal_and_mints_no_tag() {
        // Payment precedes minting in `issue`, so an admission failure
        // returns before the tag mint and before the ordinal commit.
        use super::super::expansion_budget::ExpansionLimits;
        // Budget fits the root only: the first issue fails at admission.
        let tight = ExpansionBudget::new(ExpansionLimits {
            bytes: path_bytes(0).unwrap(),
            ..ExpansionLimits::default()
        });
        let mut issuer = OwnerIssuer::new(OwnerRoot::standalone().unwrap(), &tight).unwrap();
        assert!(matches!(
            issuer.issue(OwnerDomain::TokenSession),
            Err(OwnerError::Budget(_))
        ));
        // The ordinal was not consumed: still 0 for this domain.
        assert_eq!(issuer.next[domain_index(OwnerDomain::TokenSession)], 0);
    }

    #[test]
    fn tag_exhaustion_fails_closed() {
        use std::sync::atomic::AtomicU64;
        let counter = AtomicU64::new(u64::MAX - 1);
        assert!(crate::engine::mint_from_counter_for_test(&counter).is_some());
        assert!(crate::engine::mint_from_counter_for_test(&counter).is_none());
        assert!(crate::engine::mint_from_counter_for_test(&counter).is_none());
    }

    #[test]
    fn graph_id_allocation_fails_closed_at_exhaustion() {
        use std::sync::atomic::AtomicU64;
        let counter = AtomicU64::new(u64::MAX - 1);
        assert_eq!(
            crate::engine::allocate_graph_id_for_test(&counter).map(|id| id.get()),
            Some(u64::MAX - 1)
        );
        // Issuing u64::MAX would leave no successor: refuse instead of wrapping.
        assert!(crate::engine::allocate_graph_id_for_test(&counter).is_none());
        assert!(crate::engine::allocate_graph_id_for_test(&counter).is_none());
        let zero = AtomicU64::new(0);
        assert!(crate::engine::allocate_graph_id_for_test(&zero).is_none());
    }
}
