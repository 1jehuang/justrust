//! Structural owner identities (PARSE-SPLIT.md, "Increment 1 owner derivation").
//!
//! An [`OwnerPath`] names who issued a handle: an engine authority root
//! followed by `(domain, ordinal)` steps. Every step is issued by an exclusively
//! borrowed parent [`OwnerIssuer`], so siblings get distinct ordinals and two
//! distinct issuances never share a path. Paths compare by full structure.
//! There is no hashing, truncation, pointer identity or global cache.
//!
//! Not yet activated: the existing process-global counters still issue the
//! live owner ids. This module and its tests land first so the authority
//! gates can be pinned before any `static NEXT` is removed.
#![allow(dead_code)]

use crate::EngineAuthority;
use std::sync::Arc;

/// Closed set of owner domains. A step's domain is part of its identity, so a
/// session and a module arena at the same position never compare equal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum OwnerDomain {
    /// One source file unit, keyed by its child ordinal from the crate root.
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Step {
    domain: OwnerDomain,
    ordinal: u64,
}

/// Maximum path depth. Depth is already bounded by file nesting and expansion
/// limits; this is the structural backstop, charged before allocation.
pub const MAX_OWNER_DEPTH: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerError {
    /// The parent already has `MAX_OWNER_DEPTH` steps.
    DepthExceeded,
    /// The parent issued `u64::MAX` children of one domain.
    OrdinalExhausted,
}

/// A complete, immutable owner identity. Cloning shares the step storage.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OwnerPath {
    root: EngineAuthority,
    steps: Arc<[Step]>,
}

impl OwnerPath {
    pub fn root(&self) -> EngineAuthority {
        self.root
    }
    pub fn depth(&self) -> usize {
        self.steps.len()
    }
    /// Bytes retained by this path's step storage, for paid accounting before
    /// publication. Shared clones do not add storage.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of_val::<[Step]>(&self.steps) + 2 * std::mem::size_of::<usize>()
    }
    /// Whether `self` was issued (transitively) under `ancestor`.
    pub fn descends_from(&self, ancestor: &OwnerPath) -> bool {
        self.root == ancestor.root
            && self.steps.len() >= ancestor.steps.len()
            && self.steps[..ancestor.steps.len()] == ancestor.steps[..]
    }
    /// The last step's domain, or `None` for an engine root.
    pub fn domain(&self) -> Option<OwnerDomain> {
        self.steps.last().map(|step| step.domain)
    }
}

/// Issues child owners. Holding `&mut` is the only way to issue, so ordinals
/// are assigned in one exclusive sequence per parent and domain.
#[derive(Debug)]
pub struct OwnerIssuer {
    path: OwnerPath,
    next: [u64; DOMAINS],
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

impl OwnerIssuer {
    /// The issuer for an engine root. Roots come only from an
    /// [`EngineAuthority`], which only the engine mints.
    pub fn for_engine(root: EngineAuthority) -> Self {
        Self {
            path: OwnerPath {
                root,
                steps: Arc::from([]),
            },
            next: [0; DOMAINS],
        }
    }

    /// A fresh standalone root for direct, non-graph parsing. Every call is a
    /// distinct authority, so repeated direct parses of identical text still
    /// reject each other's handles.
    pub fn standalone() -> Self {
        Self::for_engine(EngineAuthority::standalone())
    }

    pub fn path(&self) -> &OwnerPath {
        &self.path
    }

    /// Issue the next child of `domain`, returning its own issuer.
    pub fn issue(&mut self, domain: OwnerDomain) -> Result<OwnerIssuer, OwnerError> {
        let slot = &mut self.next[domain_index(domain)];
        let ordinal = *slot;
        *slot = ordinal.checked_add(1).ok_or(OwnerError::OrdinalExhausted)?;
        self.child(domain, ordinal)
    }

    /// The file at a canonical child-module ordinal. This is the only
    /// positional step: a file's identity is its place in the module tree,
    /// which is one graph node per engine. Every issued owner (sessions,
    /// arenas, drivers, slices, views) must come from [`Self::issue`].
    pub fn file(&self, ordinal: u64) -> Result<OwnerIssuer, OwnerError> {
        self.child(OwnerDomain::File, ordinal)
    }

    fn child(&self, domain: OwnerDomain, ordinal: u64) -> Result<OwnerIssuer, OwnerError> {
        if self.path.steps.len() >= MAX_OWNER_DEPTH {
            return Err(OwnerError::DepthExceeded);
        }
        let mut steps = Vec::with_capacity(self.path.steps.len() + 1);
        steps.extend_from_slice(&self.path.steps);
        steps.push(Step { domain, ordinal });
        Ok(OwnerIssuer {
            path: OwnerPath {
                root: self.path.root,
                steps: steps.into(),
            },
            next: [0; DOMAINS],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn siblings_are_distinct_and_domains_are_separated() {
        let mut root = OwnerIssuer::standalone();
        let a = root.issue(OwnerDomain::TokenSession).unwrap();
        let b = root.issue(OwnerDomain::TokenSession).unwrap();
        let m = root.issue(OwnerDomain::ModuleArena).unwrap();
        assert_ne!(a.path(), b.path());
        // Same ordinal (0) in a different domain is a different owner.
        assert_ne!(a.path(), m.path());
        assert_eq!(a.path().domain(), Some(OwnerDomain::TokenSession));
    }

    #[test]
    fn standalone_roots_never_collide() {
        let mut first = OwnerIssuer::standalone();
        let mut second = OwnerIssuer::standalone();
        let x = first.issue(OwnerDomain::TokenSession).unwrap();
        let y = second.issue(OwnerDomain::TokenSession).unwrap();
        assert_ne!(first.path().root(), second.path().root());
        assert_ne!(
            x.path(),
            y.path(),
            "identical structure under distinct roots"
        );
    }

    #[test]
    fn concurrent_standalone_roots_are_distinct() {
        let roots: Vec<_> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..16)
                .map(|_| scope.spawn(|| OwnerIssuer::standalone().path().root()))
                .collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        let unique: std::collections::BTreeSet<_> = roots.iter().collect();
        assert_eq!(unique.len(), roots.len());
    }

    #[test]
    fn nested_issue_descends_and_does_not_alias_cousins() {
        let mut root = OwnerIssuer::standalone();
        let mut file0 = root.file(0).unwrap();
        let mut file1 = root.file(1).unwrap();
        let s0 = file0.issue(OwnerDomain::TokenSession).unwrap();
        let s1 = file1.issue(OwnerDomain::TokenSession).unwrap();
        assert_ne!(s0.path(), s1.path());
        assert!(s0.path().descends_from(file0.path()));
        assert!(!s0.path().descends_from(file1.path()));
        assert!(s0.path().descends_from(root.path()));
        let _ = root.issue(OwnerDomain::SourceSnapshot).unwrap();
    }

    #[test]
    fn depth_is_bounded() {
        let mut issuer = OwnerIssuer::standalone();
        for _ in 0..MAX_OWNER_DEPTH {
            issuer = issuer.issue(OwnerDomain::TokenSlice).unwrap();
        }
        assert_eq!(
            issuer.issue(OwnerDomain::TokenSlice).unwrap_err(),
            OwnerError::DepthExceeded
        );
    }

    #[test]
    fn engine_authority_matches_context_and_differs_across_engines() {
        struct Nop;
        impl crate::Provider for Nop {
            fn outputs(&self, _: &crate::Key) -> usize {
                1
            }
            fn start(&self, cx: crate::Context, _: crate::Key) -> crate::TaskFuture {
                Box::pin(async move { cx.publish(0, 1) })
            }
        }
        let a = crate::Engine::new(Nop);
        let b = crate::Engine::new(Nop);
        assert_ne!(a.authority(), b.authority());
        assert_eq!(a.authority(), a.authority());
    }
}
