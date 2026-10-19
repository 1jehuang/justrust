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
//! the step storage is allocated. Retained step storage holds a [`ByteLease`],
//! so the charge is released when the last clone of the path drops.
//!
//! Not yet activated: the process-global counters still issue live owner ids.
#![allow(dead_code)]

use super::expansion_budget::{BudgetFailure, ByteLease, Counter, ExpansionBudget, Storage};
use crate::{EngineAuthority, OwnerRoot};
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
    Budget(BudgetFailure),
    Allocation,
}

impl From<BudgetFailure> for OwnerError {
    fn from(failure: BudgetFailure) -> Self {
        Self::Budget(failure)
    }
}

/// The producer's identity, shared by every path it issues.
#[derive(Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct RootId {
    authority: EngineAuthority,
    producer: Option<(u32, u32, Box<[u64]>)>,
}

#[derive(Debug)]
struct Steps {
    steps: Box<[Step]>,
    _lease: ByteLease,
}

/// A complete, immutable owner identity. Clones share root and step storage.
/// Equality, ordering and hashing compare the full structure.
#[derive(Clone, Debug)]
pub struct OwnerPath {
    root: Arc<RootId>,
    steps: Arc<Steps>,
}

impl PartialEq for OwnerPath {
    fn eq(&self, other: &Self) -> bool {
        self.root == other.root && self.steps.steps == other.steps.steps
    }
}
impl Eq for OwnerPath {}
impl std::hash::Hash for OwnerPath {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.root.hash(state);
        self.steps.steps.hash(state);
    }
}

impl OwnerPath {
    pub fn authority(&self) -> EngineAuthority {
        self.root.authority
    }
    pub fn depth(&self) -> usize {
        self.steps.steps.len()
    }
    /// Whether `self` was issued (transitively) under `ancestor`.
    pub fn descends_from(&self, ancestor: &OwnerPath) -> bool {
        let (mine, theirs) = (&self.steps.steps, &ancestor.steps.steps);
        self.root == ancestor.root
            && mine.len() >= theirs.len()
            && mine[..theirs.len()] == theirs[..]
    }
    pub fn domain(&self) -> Option<OwnerDomain> {
        self.steps.steps.last().map(|step| step.domain)
    }
}

/// Issues child owners from one exclusively held position in the tree.
#[derive(Debug)]
pub struct OwnerIssuer {
    path: OwnerPath,
    next: [u64; DOMAINS],
    budget: ExpansionBudget,
}

/// Retained bytes for a path of `depth` steps, including fixed `Arc` headers.
fn path_bytes(depth: usize) -> Result<u64, OwnerError> {
    let bytes = depth
        .checked_mul(std::mem::size_of::<Step>())
        .and_then(|n| {
            n.checked_add(std::mem::size_of::<Steps>() + 2 * std::mem::size_of::<usize>())
        })
        .ok_or(OwnerError::Allocation)?;
    u64::try_from(bytes).map_err(|_| OwnerError::Allocation)
}

fn paid_steps(
    budget: &ExpansionBudget,
    prefix: &[Step],
    last: Option<Step>,
) -> Result<Arc<Steps>, OwnerError> {
    let depth = prefix.len() + usize::from(last.is_some());
    if depth > MAX_OWNER_DEPTH {
        return Err(OwnerError::DepthExceeded);
    }
    // Pay for the copy and retained storage before allocating either.
    budget.charge(Counter::Work, depth as u64 + 1)?;
    let lease = budget.reserve(Storage::Retained, path_bytes(depth)?)?;
    let mut steps = Vec::new();
    steps
        .try_reserve_exact(depth)
        .map_err(|_| OwnerError::Allocation)?;
    steps.extend_from_slice(prefix);
    steps.extend(last);
    Ok(Arc::new(Steps {
        steps: steps.into_boxed_slice(),
        _lease: lease,
    }))
}

impl OwnerIssuer {
    /// The top-level issuer for one producer. Consumes the root, which is
    /// minted once per node (`Context::owner_root`) or once per standalone
    /// computation (`OwnerRoot::standalone`).
    pub fn new(root: OwnerRoot, budget: &ExpansionBudget) -> Result<Self, OwnerError> {
        let producer = root
            .producer()
            .map(|(kind, version, params)| (kind, version, params.into()));
        let params = producer
            .as_ref()
            .map_or(0, |(_, _, p): &(u32, u32, Box<[u64]>)| p.len());
        budget.charge(Counter::Work, params as u64 + 1)?;
        let root = Arc::new(RootId {
            authority: root.authority(),
            producer,
        });
        Ok(Self {
            path: OwnerPath {
                root,
                steps: paid_steps(budget, &[], None)?,
            },
            next: [0; DOMAINS],
            budget: budget.clone(),
        })
    }

    pub fn path(&self) -> &OwnerPath {
        &self.path
    }

    /// Issue the next child of `domain`. The only way to create a child.
    pub fn issue(&mut self, domain: OwnerDomain) -> Result<OwnerIssuer, OwnerError> {
        let slot = &mut self.next[domain_index(domain)];
        let ordinal = *slot;
        let following = ordinal.checked_add(1).ok_or(OwnerError::OrdinalExhausted)?;
        let steps = paid_steps(
            &self.budget,
            &self.path.steps.steps,
            Some(Step { domain, ordinal }),
        )?;
        // Commit the ordinal only after the child exists, so a failed issue
        // never skips or reuses a position.
        *slot = following;
        Ok(OwnerIssuer {
            path: OwnerPath {
                root: self.path.root.clone(),
                steps,
            },
            next: [0; DOMAINS],
            budget: self.budget.clone(),
        })
    }

    /// The path without the issuing capability, for publication. The issuer's
    /// budget handle is dropped here, so published paths carry no budget.
    pub fn into_path(self) -> OwnerPath {
        self.path
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn budget() -> ExpansionBudget {
        ExpansionBudget::default()
    }

    fn standalone() -> OwnerIssuer {
        OwnerIssuer::new(OwnerRoot::standalone().unwrap(), &budget()).unwrap()
    }

    #[test]
    fn siblings_are_distinct_and_domains_are_separated() {
        let mut root = standalone();
        let a = root.issue(OwnerDomain::TokenSession).unwrap();
        let b = root.issue(OwnerDomain::TokenSession).unwrap();
        let m = root.issue(OwnerDomain::ModuleArena).unwrap();
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
            .issue(OwnerDomain::File)
            .unwrap()
            .issue(OwnerDomain::TokenSession)
            .unwrap();
        let y = second
            .issue(OwnerDomain::File)
            .unwrap()
            .issue(OwnerDomain::TokenSession)
            .unwrap();
        assert_ne!(first.path(), second.path());
        assert_ne!(x.path(), y.path());
    }

    #[test]
    fn files_are_issued_not_positional() {
        // Two files of one parent are distinct; re-issuing never repeats a file.
        let mut root = standalone();
        let mut file0 = root.issue(OwnerDomain::File).unwrap();
        let mut file1 = root.issue(OwnerDomain::File).unwrap();
        let s0 = file0.issue(OwnerDomain::TokenSession).unwrap();
        let s1 = file1.issue(OwnerDomain::TokenSession).unwrap();
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
            issuer = issuer.issue(OwnerDomain::TokenSlice).unwrap();
        }
        assert_eq!(
            issuer.issue(OwnerDomain::TokenSlice).unwrap_err(),
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
        let _first = root.issue(OwnerDomain::TokenSession).unwrap();
        assert!(
            small.snapshot().retained_bytes > before.retained_bytes,
            "issue reserved retained bytes"
        );
        // The next child does not fit: rejected by admission, not by an
        // allocation that already happened.
        assert!(matches!(
            root.issue(OwnerDomain::TokenSession),
            Err(OwnerError::Budget(_))
        ));
        // Expansion budgets latch the first failure: no later issue succeeds.
        assert!(root.issue(OwnerDomain::ModuleArena).is_err());
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
                    let child = issuer.issue(OwnerDomain::TokenSession).expect("child");
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
