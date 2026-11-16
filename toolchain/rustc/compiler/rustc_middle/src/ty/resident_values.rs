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
use super::resident_identity::{
    Compatibility, IdentityBudget, ResetReason, ReuseAuthorization, RevisionIdentity,
};
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
    IdentityBudgetMismatch,
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

/// Cache-held typed references are bounded by the caller's actual arena domain.
/// The namespace marker has a separate lifetime: it is not an arena owner and
/// requiring it to have the arena lifetime would incorrectly demand a static
/// marker inside a higher-ranked ownership callback.
/// Only one published generation and one unpublished generation are retained. A failed
/// revision must be aborted, never published. Publication is an explicit success signal.
/// No entry, including one staged in the current revision, bypasses authorization.
/// Budget pressure may discard the entire previous published generation even if the
/// staging revision later fails. This is a cache miss, not publication of failed data.
pub struct ResidentValues<'domain, 'identity> {
    domain: &'identity ResidentDomain,
    generations: Generations<RevisionIdentity, (ResidentKey, ValueKind), TypedValue<'domain>>,
    pending_identity_budget: Option<IdentityBudget>,
    checked_identity: Option<CheckedIdentity>,
    #[cfg(test)]
    identity_checks: usize,
}

/// Private facts computed from the actual stored snapshots, never supplied by a caller.
/// Numbers alone are insufficient when analysis replaces the pending identity, so every
/// identity replacement also clears this memo. No references or extended lifetimes live here.
#[derive(Clone, Copy)]
struct CheckedIdentity {
    published: u64,
    pending: u64,
    budget: IdentityBudget,
    compatibility: Compatibility,
    leaf: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueKind {
    Typeck,
    Mir,
}

impl<'domain, 'identity> ResidentValues<'domain, 'identity> {
    pub fn new(domain: &'identity ResidentDomain, budget: RetentionBudget) -> Self {
        Self {
            domain,
            generations: Generations::new(budget),
            pending_identity_budget: None,
            checked_identity: None,
            #[cfg(test)]
            identity_checks: 0,
        }
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
        self.generations.begin(generation, identity)?;
        self.checked_identity = None;
        self.pending_identity_budget = Some(identity_budget);
        Ok(())
    }

    /// Replace only the active staging snapshot after successful analysis. This
    /// does not publish values, and cannot alter an already published generation.
    pub fn finish_pending_identity(
        &mut self,
        identity: RevisionIdentity,
        identity_budget: IdentityBudget,
    ) -> Result<(), RetentionError> {
        if self.generations.pending.is_none() {
            return Err(RetentionError::NoActiveRevision);
        }
        if self.pending_identity_budget != Some(identity_budget) {
            return Err(RetentionError::IdentityBudgetMismatch);
        }
        if identity.namespace != self.domain.namespace {
            return Err(RetentionError::NamespaceMismatch);
        }
        identity.check_budget(identity_budget).map_err(RetentionError::IdentityBudget)?;
        self.generations.pending.as_mut().unwrap().identity = identity;
        self.checked_identity = None;
        Ok(())
    }

    pub fn retain_typeck(
        &mut self,
        key: ResidentKey,
        value: &'domain TypeckResults<'domain>,
        estimated_bytes: usize,
    ) -> Result<(), RetentionError> {
        let result = self.generations.insert(
            (key, ValueKind::Typeck),
            TypedValue::Typeck(value),
            estimated_bytes,
        );
        self.invalidate_if_evicted();
        result
    }

    pub fn retain_mir(
        &mut self,
        key: ResidentKey,
        value: &'domain mir::Body<'domain>,
        estimated_bytes: usize,
    ) -> Result<(), RetentionError> {
        let result =
            self.generations.insert((key, ValueKind::Mir), TypedValue::Mir(value), estimated_bytes);
        self.invalidate_if_evicted();
        result
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

    /// Read-only candidate for closure inspection, never a query-cache hit. The
    /// caller must inspect it outside the live-state lock, then revalidate with
    /// `mir_leaf_with_proof` before returning it to a query. This path does not
    /// change or issue the ordinary same-source authorization token.
    pub(crate) fn mir_leaf_candidate(
        &mut self,
        key: ResidentKey,
        budget: IdentityBudget,
    ) -> Option<&'domain mir::Body<'domain>> {
        if !self.checked_identity(budget)?.leaf {
            return None;
        }
        let value = self.generations.get(&(key, ValueKind::Mir), |_, _| true)?;
        match value {
            TypedValue::Mir(body) => Some(*body),
            TypedValue::Typeck(_) => None,
        }
    }

    /// Recheck the actual published/current snapshot pair and exact candidate
    /// after inspecting its native closure. The opaque proof is bound to this
    /// body allocation and must still describe the current source-map view.
    pub(crate) fn mir_leaf_with_proof(
        &mut self,
        key: ResidentKey,
        budget: IdentityBudget,
        proof: &super::resident_mir_leaf::LeafProof<'_, 'domain>,
        source_map: &rustc_span::source_map::SourceMap,
    ) -> Option<&'domain mir::Body<'domain>> {
        let body = self.mir_leaf_candidate(key, budget)?;
        (proof.is_for(body) && proof.is_current_for(source_map)).then_some(body)
    }

    /// Owner-local fast path. This cannot accept caller-created compatibility facts.
    pub(crate) fn typeck_checked(
        &mut self,
        key: ResidentKey,
        budget: IdentityBudget,
    ) -> Option<&'domain TypeckResults<'domain>> {
        match self.lookup_checked((key, ValueKind::Typeck), budget)? {
            TypedValue::Typeck(value) => Some(value),
            TypedValue::Mir(_) => None,
        }
    }

    pub(crate) fn mir_checked(
        &mut self,
        key: ResidentKey,
        budget: IdentityBudget,
    ) -> Option<&'domain mir::Body<'domain>> {
        match self.lookup_checked((key, ValueKind::Mir), budget)? {
            TypedValue::Mir(value) => Some(value),
            TypedValue::Typeck(_) => None,
        }
    }

    fn lookup_checked(
        &mut self,
        key: (ResidentKey, ValueKind),
        budget: IdentityBudget,
    ) -> Option<TypedValue<'domain>> {
        if self.checked_identity(budget)?.compatibility != Compatibility::Reusable {
            return None;
        }
        self.generations.get(&key, |_, _| true).copied()
    }

    fn checked_identity(&mut self, budget: IdentityBudget) -> Option<CheckedIdentity> {
        let old = self.generations.published.as_ref()?;
        let next = self.generations.pending.as_ref()?;
        if old.number >= next.number || self.pending_identity_budget != Some(budget) {
            return None;
        }
        if let Some(checked) = self.checked_identity
            && checked.published == old.number
            && checked.pending == next.number
            && checked.budget == budget
        {
            return Some(checked);
        }
        #[cfg(test)]
        {
            self.identity_checks += 1;
        }
        let compatibility = old.identity.compatibility(&next.identity, budget);
        let checked = CheckedIdentity {
            published: old.number,
            pending: next.number,
            budget,
            compatibility,
            leaf: leaf_compatible_after_check(&old.identity, &next.identity, compatibility),
        };
        self.checked_identity = Some(checked);
        Some(checked)
    }

    fn invalidate_if_evicted(&mut self) {
        if self.generations.published.is_none() {
            self.checked_identity = None;
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
        self.generations.publish()?;
        self.checked_identity = None;
        self.pending_identity_budget = None;
        Ok(())
    }

    pub fn abort_revision(&mut self) {
        self.generations.pending = None;
        self.checked_identity = None;
        self.pending_identity_budget = None;
    }

    pub fn retained_usage(&self) -> (usize, usize) {
        self.generations.usage()
    }
}

/// Separate, deliberately local-only identity gate for a primitive MIR closure
/// proof. Ordinary `authorize_reuse` remains unchanged and rejects source edits.
/// Complete numeric mappings must agree, and body/semantic inputs other than the
/// active source snapshot must be byte-identical. A source difference alone is
/// not authorization: the exact selected native body still needs `LeafProof`.
#[cfg(test)]
fn leaf_identity_compatible(
    old: &RevisionIdentity,
    next: &RevisionIdentity,
    budget: IdentityBudget,
) -> bool {
    leaf_compatible_after_check(old, next, old.compatibility(next, budget))
}

fn leaf_compatible_after_check(
    old: &RevisionIdentity,
    next: &RevisionIdentity,
    compatibility: Compatibility,
) -> bool {
    use super::resident_identity::RevisionGate;
    matches!(compatibility, Compatibility::IdentityOnly(RevisionGate::SourceChanged))
        && old.namespace == b"justrust-local-same-source-v1"
        && old.crates.len() == 1
        && old.crates[0].crate_index == 0
        && old.definitions.iter().all(|definition| definition.crate_index == 0)
        && old.body_revision == next.body_revision
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

    fn memo_identity() -> RevisionIdentity {
        use super::super::resident_identity::{CrateIdentity, DefinitionIdentity};
        RevisionIdentity {
            namespace: b"justrust-local-same-source-v1".to_vec(),
            options: b"options".to_vec(),
            crates: vec![CrateIdentity { stable: b"local".to_vec(), crate_index: 0 }],
            definitions: vec![DefinitionIdentity {
                stable: b"root".to_vec(),
                crate_index: 0,
                def_index: 0,
                owner_index: Some(0),
            }],
            source: b"source".to_vec(),
            body_revision: b"body".to_vec(),
        }
    }

    const MEMO_BUDGET: IdentityBudget =
        IdentityBudget { max_crates: 2, max_definitions: 8, max_identity_bytes: 4096 };

    #[test]
    fn memo_checks_once_and_rechecks_replaced_identity_in_the_same_generation() {
        use super::{Compatibility, ResidentDomain, ResidentValues};
        let identity = memo_identity();
        let domain = ResidentDomain::new(identity.namespace.clone());
        let mut values = ResidentValues::new(
            &domain,
            RetentionBudget { max_entries: 8, max_estimated_bytes: 4096 },
        );
        assert!(values.checked_identity(MEMO_BUDGET).is_none());
        values.begin_revision(1, identity.clone(), MEMO_BUDGET).unwrap();
        assert!(values.checked_identity(MEMO_BUDGET).is_none());
        values.publish_success().unwrap();
        values.begin_revision(2, identity.clone(), MEMO_BUDGET).unwrap();
        for _ in 0..128 {
            let checked = values.checked_identity(MEMO_BUDGET).unwrap();
            assert_eq!(checked.compatibility, Compatibility::Reusable);
            assert!(!checked.leaf);
            assert_eq!((checked.published, checked.pending), (1, 2));
        }
        assert_eq!(values.identity_checks, 1);
        let wrong_budget = IdentityBudget { max_identity_bytes: 1, ..MEMO_BUDGET };
        assert!(values.checked_identity(wrong_budget).is_none());
        assert_eq!(values.identity_checks, 1);

        let mut changed = identity.clone();
        changed.source.push(0);
        values.finish_pending_identity(changed.clone(), MEMO_BUDGET).unwrap();
        assert!(values.checked_identity.is_none());
        for _ in 0..128 {
            let checked = values.checked_identity(MEMO_BUDGET).unwrap();
            assert!(checked.leaf);
            assert_ne!(checked.compatibility, Compatibility::Reusable);
        }
        assert_eq!(values.identity_checks, 2);
        changed.body_revision.push(0);
        values.finish_pending_identity(changed, MEMO_BUDGET).unwrap();
        assert!(!values.checked_identity(MEMO_BUDGET).unwrap().leaf);
        assert_eq!(values.identity_checks, 3);
        values.finish_pending_identity(identity.clone(), MEMO_BUDGET).unwrap();
        assert_eq!(
            values.checked_identity(MEMO_BUDGET).unwrap().compatibility,
            Compatibility::Reusable
        );
        values.abort_revision();
        assert!(values.checked_identity.is_none());
        assert!(values.checked_identity(MEMO_BUDGET).is_none());
        values.begin_revision(3, identity, MEMO_BUDGET).unwrap();
        assert!(values.checked_identity.is_none());
        assert_eq!(values.checked_identity(MEMO_BUDGET).unwrap().pending, 3);
        values.publish_success().unwrap();
        assert!(values.checked_identity.is_none());
        assert!(values.checked_identity(MEMO_BUDGET).is_none());
    }

    #[test]
    fn memo_caches_rejection_and_never_accepts_equal_malformed_snapshots() {
        use super::{Compatibility, ResidentDomain, ResidentValues};
        for mutation in 0..5 {
            let mut identity = memo_identity();
            match mutation {
                0 => identity.crates.push(identity.crates[0].clone()),
                1 => identity.definitions.push(identity.definitions[0].clone()),
                2 => identity.definitions[0].crate_index = 9,
                _ => {}
            }
            let domain = ResidentDomain::new(identity.namespace.clone());
            let mut values = ResidentValues::new(
                &domain,
                RetentionBudget { max_entries: 8, max_estimated_bytes: 4096 },
            );
            values.begin_revision(1, identity.clone(), MEMO_BUDGET).unwrap();
            values.publish_success().unwrap();
            let mut next = identity;
            match mutation {
                3 => next.options.push(0),
                4 => next.definitions[0].def_index += 1,
                _ => {}
            }
            next.source.push(0);
            values.begin_revision(2, next, MEMO_BUDGET).unwrap();
            for _ in 0..128 {
                let checked = values.checked_identity(MEMO_BUDGET).unwrap();
                assert!(matches!(checked.compatibility, Compatibility::Reset(_)));
                assert!(!checked.leaf);
            }
            assert_eq!(values.identity_checks, 1);
        }
    }

    #[test]
    fn memo_invalidation_after_publication_eviction_prevents_all_lookup_paths() {
        use super::{ResidentDomain, ResidentKey, ResidentValues, ValueKind};
        use crate::dep_graph::{DepKind, DepNode};
        let identity = memo_identity();
        let domain = ResidentDomain::new(identity.namespace.clone());
        let mut values = ResidentValues::new(
            &domain,
            RetentionBudget { max_entries: 1, max_estimated_bytes: 4096 },
        );
        // The generic pressure test below covers actual insert-driven eviction.
        // Here exercise the post-eviction memo invalidator without fake typed values.
        values.begin_revision(1, identity.clone(), MEMO_BUDGET).unwrap();
        values.publish_success().unwrap();
        values.begin_revision(2, identity, MEMO_BUDGET).unwrap();
        assert!(values.checked_identity(MEMO_BUDGET).is_some());
        let key = ResidentKey {
            dep_node: DepNode {
                kind: DepKind::typeck_root,
                key_fingerprint: super::Fingerprint::new(1, 2).into(),
            },
            value_fingerprint: super::Fingerprint::new(3, 4),
        };
        values.generations.published = None;
        values.invalidate_if_evicted();
        assert!(values.checked_identity.is_none());
        assert!(values.lookup_checked((key, ValueKind::Typeck), MEMO_BUDGET).is_none());
        assert!(values.mir_leaf_candidate(key, MEMO_BUDGET).is_none());
    }

    #[test]
    fn leaf_identity_gate_is_separate_and_never_ignores_other_semantic_changes() {
        use super::super::resident_identity::{CrateIdentity, DefinitionIdentity};
        let budget = IdentityBudget { max_crates: 2, max_definitions: 8, max_identity_bytes: 4096 };
        let old = RevisionIdentity {
            namespace: b"justrust-local-same-source-v1".to_vec(),
            options: b"exact invocation".to_vec(),
            crates: vec![CrateIdentity { stable: b"native local crate".to_vec(), crate_index: 0 }],
            definitions: vec![DefinitionIdentity {
                stable: b"native local root".to_vec(),
                crate_index: 0,
                def_index: 0,
                owner_index: Some(0),
            }],
            source: b"previous exact source snapshot".to_vec(),
            body_revision: b"exact non-source inputs".to_vec(),
        };
        let mut changed = old.clone();
        changed.source = b"next exact source snapshot".to_vec();
        assert!(super::leaf_identity_compatible(&old, &changed, budget));
        assert!(old.authorize_reuse(&changed, budget).is_err());
        assert!(!super::leaf_identity_compatible(&old, &old, budget));
        for mutate in [
            |id: &mut RevisionIdentity| id.namespace.push(0),
            |id: &mut RevisionIdentity| id.options.push(0),
            |id: &mut RevisionIdentity| id.body_revision.push(0),
            |id: &mut RevisionIdentity| id.crates[0].stable.push(0),
            |id: &mut RevisionIdentity| id.crates[0].crate_index = 1,
            |id: &mut RevisionIdentity| id.definitions[0].stable.push(0),
            |id: &mut RevisionIdentity| id.definitions[0].def_index = 1,
            |id: &mut RevisionIdentity| id.definitions.push(id.definitions[0].clone()),
        ] {
            let mut invalid = changed.clone();
            mutate(&mut invalid);
            assert!(!super::leaf_identity_compatible(&old, &invalid, budget));
        }
        let mut external = old.clone();
        external.crates[0].crate_index = 1;
        external.definitions[0].crate_index = 1;
        let mut external_changed = external.clone();
        external_changed.source.push(0);
        assert!(!super::leaf_identity_compatible(&external, &external_changed, budget));
        assert!(!super::leaf_identity_compatible(
            &old,
            &changed,
            IdentityBudget { max_identity_bytes: 1, ..budget },
        ));
    }

    #[test]
    fn completion_validates_namespace_and_original_budget_without_touching_publication() {
        use super::{ResidentDomain, ResidentValues};

        let marker = ResidentDomain::new(b"domain".to_vec());
        let mut values = ResidentValues::new(
            &marker,
            RetentionBudget { max_entries: 4, max_estimated_bytes: 4096 },
        );
        let identity = RevisionIdentity {
            namespace: b"domain".to_vec(),
            options: Vec::new(),
            crates: Vec::new(),
            definitions: Vec::new(),
            source: b"source".to_vec(),
            body_revision: Vec::new(),
        };
        let budget = IdentityBudget { max_crates: 0, max_definitions: 0, max_identity_bytes: 128 };
        assert_eq!(
            values.finish_pending_identity(identity.clone(), budget),
            Err(RetentionError::NoActiveRevision)
        );
        values.begin_revision(1, identity.clone(), budget).unwrap();
        values.publish_success().unwrap();
        values.begin_revision(2, identity.clone(), budget).unwrap();
        let mut wrong = identity.clone();
        wrong.namespace.push(b'!');
        assert_eq!(
            values.finish_pending_identity(wrong, budget),
            Err(RetentionError::NamespaceMismatch)
        );
        let wrong_budget = IdentityBudget { max_identity_bytes: 256, ..budget };
        assert_eq!(
            values.finish_pending_identity(identity.clone(), wrong_budget),
            Err(RetentionError::IdentityBudgetMismatch)
        );
        let mut oversized = identity.clone();
        oversized.options = vec![0; 129];
        assert!(matches!(
            values.finish_pending_identity(oversized, budget),
            Err(RetentionError::IdentityBudget(_))
        ));
        let mut completed = identity.clone();
        completed.options = b"completed snapshot".to_vec();
        values.finish_pending_identity(completed.clone(), budget).unwrap();
        let (old, next) = values.revision_identities().unwrap();
        assert_eq!(old, &identity);
        assert_eq!(next, &completed);
        values.abort_revision();
        assert_eq!(&values.generations.published.as_ref().unwrap().identity, &identity);
        assert_eq!(
            values.finish_pending_identity(completed, budget),
            Err(RetentionError::NoActiveRevision)
        );
    }

    #[test]
    fn actual_typeck_pointer_survives_successful_revision_publication() {
        use std::num::NonZero;

        use rustc_data_structures::sync::{Registry, set_dyn_thread_safe_mode};
        use rustc_hir::CRATE_OWNER_ID;

        use super::{ResidentDomain, ResidentKey, ResidentValues, TypeckResults};
        use crate::dep_graph::{DepKind, DepNode};
        use crate::ty::resident_identity::{CrateIdentity, DefinitionIdentity};
        use crate::ty::with_resident_type_domain;

        // Initialize the parallel unit-test process's native interner mode.
        // The setter is one-way and cannot toggle an existing compiler mode.
        set_dyn_thread_safe_mode(true);
        // Do not replace another test's worker registry or session globals.
        std::thread::scope(|scope| {
            scope
                .spawn(|| {
                    Registry::new(NonZero::new(1).unwrap()).register();
                    rustc_span::create_default_session_globals_then(|| {
                        // This marker must outlive the arena callback, not the reverse.
                        let owner = ResidentDomain::new(b"actual-typeck-domain".to_vec());
                        with_resident_type_domain(|domain| {
                            let identity = RevisionIdentity {
                                namespace: b"actual-typeck-domain".to_vec(),
                                options: b"test options".to_vec(),
                                crates: vec![CrateIdentity {
                                    stable: b"test crate".to_vec(),
                                    crate_index: 0,
                                }],
                                definitions: vec![DefinitionIdentity {
                                    stable: b"test crate root".to_vec(),
                                    crate_index: 0,
                                    def_index: 0,
                                    owner_index: Some(0),
                                }],
                                source: b"fn main() {}".to_vec(),
                                body_revision: b"unchanged body".to_vec(),
                            };
                            let identity_budget = IdentityBudget {
                                max_crates: 1,
                                max_definitions: 1,
                                max_identity_bytes: 1024,
                            };
                            let mut cache = ResidentValues::new(
                                &owner,
                                RetentionBudget {
                                    max_entries: 2,
                                    max_estimated_bytes: 1024 * 1024,
                                },
                            );
                            let key = ResidentKey {
                                dep_node: DepNode {
                                    kind: DepKind::typeck_root,
                                    key_fingerprint: super::Fingerprint::new(1u64, 2u64).into(),
                                },
                                value_fingerprint: super::Fingerprint::new(3u64, 4u64),
                            };
                            let mut typeck = TypeckResults::new(CRATE_OWNER_ID);
                            typeck
                                .node_types_mut()
                                .insert(rustc_hir::CRATE_HIR_ID, domain.bool_type());
                            let actual = domain.alloc_typeck(typeck);
                            cache.begin_revision(1, identity.clone(), identity_budget).unwrap();
                            cache
                                .retain_typeck(key, actual, std::mem::size_of_val(actual))
                                .unwrap();
                            cache.publish_success().unwrap();
                            cache.begin_revision(2, identity, identity_budget).unwrap();
                            {
                                let (old, next) = cache.revision_identities().unwrap();
                                let authorization =
                                    old.authorize_reuse(next, identity_budget).unwrap();
                                let hit = cache.typeck(key, &authorization).unwrap();
                                assert!(std::ptr::eq(hit, actual));
                                assert_eq!(hit.hir_owner, CRATE_OWNER_ID);
                                assert_eq!(
                                    hit.node_types().get(rustc_hir::CRATE_HIR_ID),
                                    Some(&domain.bool_type())
                                );
                                let mut changed = next.clone();
                                changed.source.push(b' ');
                                assert!(old.authorize_reuse(&changed, identity_budget).is_err());
                            }
                            for _ in 0..128 {
                                assert!(std::ptr::eq(
                                    cache.typeck_checked(key, identity_budget).unwrap(),
                                    actual,
                                ));
                                assert!(cache.mir_checked(key, identity_budget).is_none());
                            }
                            assert_eq!(cache.identity_checks, 1);
                            let (old, next) = cache.revision_identities().unwrap();
                            let external_old = old.clone();
                            let external_next = next.clone();
                            let external_token = external_old
                                .authorize_reuse(&external_next, identity_budget)
                                .unwrap();
                            assert!(cache.typeck(key, &external_token).is_none());
                            cache.abort_revision();
                            let mut changed =
                                cache.generations.published.as_ref().unwrap().identity.clone();
                            changed.source.push(b' ');
                            cache.begin_revision(3, changed, identity_budget).unwrap();
                            let (old, next) = cache.revision_identities().unwrap();
                            assert!(old.authorize_reuse(next, identity_budget).is_err());
                            assert!(cache.typeck_checked(key, identity_budget).is_none());
                            assert!(cache.checked_identity.is_some());
                            cache.generations.budget.max_entries = 1;
                            cache
                                .retain_typeck(key, actual, std::mem::size_of_val(actual))
                                .unwrap();
                            assert!(cache.generations.published.is_none());
                            assert!(cache.checked_identity.is_none());
                            assert!(cache.typeck_checked(key, identity_budget).is_none());
                        });
                    });
                })
                .join()
                .unwrap();
        });
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
