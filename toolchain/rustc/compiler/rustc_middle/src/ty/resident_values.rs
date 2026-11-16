//! Lifetime-scoped live typed retention prerequisite. **No runtime query hits yet.**
//!
//! This module is deliberately not wired into the query engine. Its caller must first
//! supply a true common arena/interner domain covering every retained revision. A marker
//! is not evidence that independent `TyCtxt`s share such a domain. All values remain
//! ordinary Rust borrows: this module neither extends lifetimes nor serializes values.
//! Bounds account for caller-estimated reachable bytes, not exact arena allocation or
//! reclamation. Evicting references does not free caller-owned arena allocations or
//! revoke references already returned to callers. Bounds cover cache-held entries only.

use rustc_data_structures::fingerprint::Fingerprint;

use super::TypeckResults;
use super::resident_identity::{IdentityBudget, ResetReason, ReuseAuthorization, RevisionIdentity};
use crate::dep_graph::DepNode;
use crate::mir;

/// Caller-owned domain identity. Keep this alongside the common arenas/interner.
/// Exact namespace bytes, rather than their digest, distinguish domains.
pub struct ResidentDomain {
    namespace: Vec<u8>,
}

impl ResidentDomain {
    pub fn new(namespace: Vec<u8>) -> Self {
        Self { namespace }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RetentionBudget {
    pub max_entries: usize,
    pub max_estimated_bytes: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RetentionError {
    RevisionNotIncreasing,
    RevisionAlreadyActive,
    NoActiveRevision,
    NamespaceMismatch,
    IdentityBudget(ResetReason),
    Oversize,
    DuplicateKey,
}

/// Both the dependency identity and the computed value identity must match.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ResidentKey {
    pub dep_node: DepNode,
    pub value_fingerprint: Fingerprint,
}

#[derive(Clone, Copy)]
enum TypedValue<'domain> {
    Typeck(&'domain TypeckResults<'domain>),
    Mir(&'domain mir::Body<'domain>),
}

/// Cache ownership and all retained references are bounded by the caller's domain.
/// Only one published generation and one unpublished generation are retained. A failed
/// revision must be aborted, never published. Publication is an explicit success signal.
/// No entry, including one staged in the current revision, bypasses authorization.
/// Budget pressure may discard the entire previous published generation even if the
/// staging revision later fails. This is a cache miss, not publication of failed data.
pub struct ResidentValues<'domain> {
    domain: &'domain ResidentDomain,
    generations: Generations<RevisionIdentity, (ResidentKey, ValueKind), TypedValue<'domain>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Typeck,
    Mir,
}

impl<'domain> ResidentValues<'domain> {
    pub fn new(domain: &'domain ResidentDomain, budget: RetentionBudget) -> Self {
        Self { domain, generations: Generations::new(budget) }
    }

    pub fn begin_revision(
        &mut self,
        generation: u64,
        identity: RevisionIdentity,
        identity_budget: IdentityBudget,
    ) -> Result<(), RetentionError> {
        if identity.namespace != self.domain.namespace {
            return Err(RetentionError::NamespaceMismatch);
        }
        identity.check_budget(identity_budget).map_err(RetentionError::IdentityBudget)?;
        self.generations.begin(generation, identity)
    }

    pub fn retain_typeck(
        &mut self,
        key: ResidentKey,
        value: &'domain TypeckResults<'domain>,
        estimated_bytes: usize,
    ) -> Result<(), RetentionError> {
        self.generations.insert(
            (key, ValueKind::Typeck),
            TypedValue::Typeck(value),
            estimated_bytes,
        )
    }

    pub fn retain_mir(
        &mut self,
        key: ResidentKey,
        value: &'domain mir::Body<'domain>,
        estimated_bytes: usize,
    ) -> Result<(), RetentionError> {
        self.generations.insert((key, ValueKind::Mir), TypedValue::Mir(value), estimated_bytes)
    }

    /// Borrow the exact published/current snapshots for the caller's explicit identity
    /// validation. Tokens made from external clones cannot authorize these generations.
    /// The resulting borrow prevents revision mutation while authorization is in use.
    pub fn revision_identities(&self) -> Option<(&RevisionIdentity, &RevisionIdentity)> {
        Some((
            &self.generations.published.as_ref()?.identity,
            &self.generations.pending.as_ref()?.identity,
        ))
    }

    pub fn typeck(
        &self,
        key: ResidentKey,
        authorization: &ReuseAuthorization<'_>,
    ) -> Option<&'domain TypeckResults<'domain>> {
        match self.lookup((key, ValueKind::Typeck), authorization)? {
            TypedValue::Typeck(value) => Some(value),
            TypedValue::Mir(_) => None,
        }
    }

    pub fn mir(
        &self,
        key: ResidentKey,
        authorization: &ReuseAuthorization<'_>,
    ) -> Option<&'domain mir::Body<'domain>> {
        match self.lookup((key, ValueKind::Mir), authorization)? {
            TypedValue::Mir(value) => Some(value),
            TypedValue::Typeck(_) => None,
        }
    }

    fn lookup(
        &self,
        key: (ResidentKey, ValueKind),
        authorization: &ReuseAuthorization<'_>,
    ) -> Option<TypedValue<'domain>> {
        self.generations.get(&key, |old, next| authorization.authorizes(old, next)).copied()
    }

    /// Call only after the complete revision has succeeded, including diagnostics.
    pub fn publish_success(&mut self) -> Result<(), RetentionError> {
        self.generations.publish()
    }

    pub fn abort_revision(&mut self) {
        self.generations.pending = None;
    }

    pub fn retained_usage(&self) -> (usize, usize) {
        self.generations.usage()
    }
}

struct Entry<K, V> {
    key: K,
    value: V,
}

struct Generation<I, K, V> {
    number: u64,
    identity: I,
    entries: Vec<Entry<K, V>>,
    bytes: usize,
}

/// Generic state machine enables real publication/bounds tests without fake compiler
/// references. Linear lookup keeps this prerequisite small, with bounded entry count.
struct Generations<I, K, V> {
    budget: RetentionBudget,
    last_started: Option<u64>,
    published: Option<Generation<I, K, V>>,
    pending: Option<Generation<I, K, V>>,
}

impl<I, K: PartialEq, V> Generations<I, K, V> {
    fn new(budget: RetentionBudget) -> Self {
        Self { budget, last_started: None, published: None, pending: None }
    }

    fn begin(&mut self, number: u64, identity: I) -> Result<(), RetentionError> {
        if self.pending.is_some() {
            return Err(RetentionError::RevisionAlreadyActive);
        }
        if self.last_started.is_some_and(|last| number <= last) {
            return Err(RetentionError::RevisionNotIncreasing);
        }
        self.last_started = Some(number);
        self.pending = Some(Generation { number, identity, entries: Vec::new(), bytes: 0 });
        Ok(())
    }

    fn usage(&self) -> (usize, usize) {
        self.published
            .iter()
            .chain(self.pending.iter())
            .fold((0, 0), |(entries, bytes), g| (entries + g.entries.len(), bytes + g.bytes))
    }

    fn insert(&mut self, key: K, value: V, bytes: usize) -> Result<(), RetentionError> {
        let pending = self.pending.as_ref().ok_or(RetentionError::NoActiveRevision)?;
        if pending.entries.iter().any(|entry| entry.key == key) {
            return Err(RetentionError::DuplicateKey);
        }
        // Reject a single oversized value or an oversized whole staging generation
        // before evicting anything. Subtraction avoids overflow, including usize::MAX.
        if pending.entries.len() >= self.budget.max_entries
            || bytes > self.budget.max_estimated_bytes.saturating_sub(pending.bytes)
        {
            return Err(RetentionError::Oversize);
        }
        let (entries, retained_bytes) = self.usage();
        if entries >= self.budget.max_entries
            || bytes > self.budget.max_estimated_bytes.saturating_sub(retained_bytes)
        {
            // Evict only whole generations, never a subset of a published revision.
            self.published = None;
        }
        let pending = self.pending.as_mut().unwrap();
        pending.bytes += bytes;
        pending.entries.push(Entry { key, value });
        Ok(())
    }

    fn get(&self, key: &K, authorize: impl FnOnce(&I, &I) -> bool) -> Option<&V> {
        let old = self.published.as_ref()?;
        let next = self.pending.as_ref()?;
        if old.number >= next.number || !authorize(&old.identity, &next.identity) {
            return None;
        }
        old.entries.iter().find(|entry| &entry.key == key).map(|entry| &entry.value)
    }

    fn publish(&mut self) -> Result<(), RetentionError> {
        let pending = self.pending.take().ok_or(RetentionError::NoActiveRevision)?;
        self.published = Some(pending);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{Generations, IdentityBudget, RetentionBudget, RetentionError, RevisionIdentity};

    fn cache(entries: usize, bytes: usize) -> Generations<&'static str, u32, &'static str> {
        Generations::new(RetentionBudget { max_entries: entries, max_estimated_bytes: bytes })
    }

    #[test]
    fn actual_identity_token_is_bound_to_stored_snapshots() {
        let identity = RevisionIdentity {
            namespace: b"domain".to_vec(),
            options: b"options".to_vec(),
            crates: Vec::new(),
            definitions: Vec::new(),
            source: b"source".to_vec(),
            body_revision: b"body".to_vec(),
        };
        let budget = IdentityBudget { max_crates: 0, max_definitions: 0, max_identity_bytes: 100 };
        let mut c = Generations::new(RetentionBudget { max_entries: 2, max_estimated_bytes: 100 });
        c.begin(1, identity.clone()).unwrap();
        c.insert(1, "typed", 1).unwrap();
        c.publish().unwrap();
        c.begin(2, identity.clone()).unwrap();
        let old = &c.published.as_ref().unwrap().identity;
        let next = &c.pending.as_ref().unwrap().identity;
        let token = old.authorize_reuse(next, budget).unwrap();
        assert_eq!(c.get(&1, |old, next| token.authorizes(old, next)), Some(&"typed"));
        let clone = next.clone();
        let wrong = identity.authorize_reuse(&clone, budget).unwrap();
        assert_eq!(c.get(&1, |old, next| wrong.authorizes(old, next)), None);
        let mut changed = next.clone();
        changed.source.push(b' ');
        assert!(old.authorize_reuse(&changed, budget).is_err());
    }

    #[test]
    fn only_previous_success_with_explicit_matching_authorization_hits() {
        let mut c = cache(8, 100);
        c.begin(1, "domain-a").unwrap();
        c.insert(7, "typed value", 10).unwrap();
        assert_eq!(c.get(&7, |_, _| true), None);
        c.publish().unwrap();
        assert_eq!(c.get(&7, |_, _| true), None);
        c.begin(2, "domain-a").unwrap();
        assert_eq!(c.get(&7, |old, next| old == next), Some(&"typed value"));
        assert_eq!(c.get(&7, |_, _| false), None);
        assert_eq!(c.get(&8, |_, _| true), None);
        c.pending.as_mut().unwrap().identity = "domain-b";
        assert_eq!(c.get(&7, |old, next| old == next), None);
    }

    #[test]
    fn failed_revision_is_not_published_and_generations_never_rewind() {
        let mut c = cache(8, 100);
        c.begin(10, "a").unwrap();
        c.insert(1, "successful", 1).unwrap();
        c.publish().unwrap();
        c.begin(11, "a").unwrap();
        c.insert(2, "failed", 1).unwrap();
        c.pending = None;
        assert_eq!(c.publish(), Err(RetentionError::NoActiveRevision));
        assert_eq!(c.begin(11, "a"), Err(RetentionError::RevisionNotIncreasing));
        c.begin(12, "a").unwrap();
        assert_eq!(c.get(&1, |_, _| true), Some(&"successful"));
        assert_eq!(c.get(&2, |_, _| true), None);
        assert_eq!(c.begin(13, "a"), Err(RetentionError::RevisionAlreadyActive));
    }

    #[test]
    fn pressure_evicts_whole_published_generation_and_rejects_oversize() {
        for (entries, bytes) in [(2, 100), (10, 2)] {
            let mut c = cache(entries, bytes);
            c.begin(1, "a").unwrap();
            c.insert(1, "one", 1).unwrap();
            c.insert(2, "two", 1).unwrap();
            c.publish().unwrap();
            c.begin(2, "a").unwrap();
            c.insert(3, "three", 1).unwrap();
            assert!(c.published.is_none());
            assert_eq!(c.usage(), (1, 1));
            assert_eq!(c.get(&1, |_, _| true), None);
            assert_eq!(c.get(&2, |_, _| true), None);
            assert_eq!(c.insert(4, "too big", usize::MAX), Err(RetentionError::Oversize));
            assert_eq!(c.usage(), (1, 1));
        }
    }

    #[test]
    fn publication_replaces_previous_generation_not_a_union() {
        let mut c = cache(8, 100);
        c.begin(1, "a").unwrap();
        c.insert(1, "old", 1).unwrap();
        c.publish().unwrap();
        c.begin(2, "a").unwrap();
        c.insert(2, "new", 1).unwrap();
        assert_eq!(c.get(&2, |_, _| true), None);
        c.publish().unwrap();
        c.begin(3, "a").unwrap();
        assert_eq!(c.get(&1, |_, _| true), None);
        assert_eq!(c.get(&2, |_, _| true), Some(&"new"));
        assert_eq!(c.usage(), (1, 1));
    }

    #[test]
    fn both_dependency_and_value_identity_must_match() {
        let mut c = Generations::new(RetentionBudget { max_entries: 8, max_estimated_bytes: 100 });
        c.begin(1, "a").unwrap();
        c.insert((10, 20), "typed", 1).unwrap();
        c.publish().unwrap();
        c.begin(2, "a").unwrap();
        assert_eq!(c.get(&(10, 20), |_, _| true), Some(&"typed"));
        assert_eq!(c.get(&(10, 21), |_, _| true), None);
        assert_eq!(c.get(&(11, 20), |_, _| true), None);
    }

    #[test]
    fn rejects_zero_entry_budget_duplicates_and_staging_overflow() {
        let mut c = cache(0, usize::MAX);
        c.begin(1, "a").unwrap();
        assert_eq!(c.insert(1, "x", 0), Err(RetentionError::Oversize));
        let mut c = cache(2, usize::MAX);
        assert_eq!(c.insert(1, "x", 1), Err(RetentionError::NoActiveRevision));
        c.begin(1, "a").unwrap();
        c.insert(1, "x", usize::MAX).unwrap();
        assert_eq!(c.insert(1, "other", 0), Err(RetentionError::DuplicateKey));
        assert_eq!(c.insert(2, "y", 1), Err(RetentionError::Oversize));
        assert_eq!(c.usage(), (1, usize::MAX));
        assert_eq!(c.pending.as_ref().unwrap().bytes, usize::MAX);
    }
}
