//! Native, owner-scoped query values for the explicit local same-source pipeline.
//!
//! Nothing starts implicitly. Completed HIR identity must be supplied before
//! analysis, completed identity after analysis, and transport success only after
//! codegen/link and diagnostic finalization. No runtime hits are claimed here.

use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::sync::Lock;

use super::resident_identity::{IdentityBudget, RevisionIdentity};
use super::resident_values::{
    ResidentDomain, ResidentKey, ResidentValues, RetentionBudget, RetentionError,
};
use super::{TyCtxt, TypeckResults};
use crate::dep_graph::DepNode;
use crate::mir;

/// Exact namespace of the factory-owned, controlled local-only first milestone.
pub const LIVE_QUERY_NAMESPACE: &[u8] = b"justrust-local-same-source-v1";

/// Fixed-size accounting, with no query names, keys, or compiler-owned values.
/// Reasons describe the first guard that refused an operation, not speculation
/// about why a provider ran. All counters saturate rather than overflowing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveQueryRefusals {
    pub disabled: u64,
    pub identity_absent: u64,
    pub dependency: u64,
    pub diagnostic: u64,
    pub ineligible: u64,
    pub retention: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveQueryRefusal {
    Disabled,
    IdentityAbsent,
    Dependency,
    Diagnostic,
    Ineligible,
    Retention,
}

impl LiveQueryRefusals {
    fn record(&mut self, reason: LiveQueryRefusal) {
        let counter = match reason {
            LiveQueryRefusal::Disabled => &mut self.disabled,
            LiveQueryRefusal::IdentityAbsent => &mut self.identity_absent,
            LiveQueryRefusal::Dependency => &mut self.dependency,
            LiveQueryRefusal::Diagnostic => &mut self.diagnostic,
            LiveQueryRefusal::Ineligible => &mut self.ineligible,
            LiveQueryRefusal::Retention => &mut self.retention,
        };
        *counter = counter.saturating_add(1);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveQueryKind {
    Typeck,
    Mir,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveQueryStats {
    pub typeck_admitted: u64,
    pub mir_admitted: u64,
    pub typeck_admission_refusals: LiveQueryRefusals,
    pub mir_admission_refusals: LiveQueryRefusals,
    /// Exact last storage error, never synthesized from a cache miss.
    pub last_retention_error: Option<RetentionError>,
    pub publication_disabled: u64,
    pub publication_transport_failed: u64,
    pub publication_incomplete: u64,
    pub publication_storage_failed: u64,
    /// Revision-level gates, distinct from per-query cache misses.
    pub identity_begin_rejections: u64,
    pub identity_complete_rejections: u64,
    pub published_revisions: u64,
    pub aborted_revisions: u64,
    pub typeck_hits: u64,
    pub typeck_misses: u64,
    pub mir_hits: u64,
    /// A distinct counter for the separate primitive-leaf source-closure path.
    pub mir_leaf_hits: u64,
    pub mir_leaf_candidate_misses: u64,
    pub mir_leaf_last_rejection: Option<super::resident_mir_leaf::LeafError>,
    pub mir_misses: u64,
    pub retained_entries: usize,
    pub retained_estimated_bytes: usize,
}

struct LiveState<'tcx> {
    enabled: bool,
    leaf_enabled: bool,
    values: ResidentValues<'tcx, 'tcx>,
    identity_budget: Option<IdentityBudget>,
    completed: bool,
    stats: LiveQueryStats,
}

/// Borrowed only within the factory's higher-ranked arena ownership scope.
/// The lock protects actual typed references, not encoded proxies.
pub struct LiveQueryState<'tcx> {
    state: Lock<LiveState<'tcx>>,
}

impl<'tcx> LiveQueryState<'tcx> {
    pub fn new(domain: &'tcx ResidentDomain, budget: RetentionBudget) -> Self {
        Self {
            state: Lock::new(LiveState {
                enabled: false,
                leaf_enabled: false,
                values: ResidentValues::new(domain, budget),
                identity_budget: None,
                completed: false,
                stats: LiveQueryStats::default(),
            }),
        }
    }

    pub fn enabled(&self) -> bool {
        self.state.lock().enabled
    }

    /// Explicit opt-in only. Disabling aborts any unfinished revision.
    pub fn enable(&self, enabled: bool) {
        let mut state = self.state.lock();
        state.enabled = enabled;
        if !enabled {
            state.values.abort_revision();
            state.identity_budget = None;
            state.completed = false;
        }
    }

    /// Separate experimental opt-in, never implied by the same-source mode.
    pub fn enable_leaf(&self, enabled: bool) {
        self.state.lock().leaf_enabled = enabled;
    }

    /// Called by the pipeline only after HIR is complete, before analysis.
    /// Identity failures disable this revision's retention, not compilation.
    pub fn begin(
        &self,
        generation: u64,
        identity: Option<RevisionIdentity>,
        budget: IdentityBudget,
    ) -> bool {
        let mut state = self.state.lock();
        state.values.abort_revision();
        state.identity_budget = None;
        state.completed = false;
        if !state.enabled {
            return false;
        }
        let Some(identity) = identity else {
            state.stats.identity_begin_rejections += 1;
            return false;
        };
        if state.values.begin_revision(generation, identity, budget).is_err() {
            state.stats.identity_begin_rejections += 1;
            return false;
        }
        state.identity_budget = Some(budget);
        true
    }

    /// Analysis succeeded. Replace the staging identity, but do not publish.
    pub fn complete(&self, identity: Option<RevisionIdentity>, budget: IdentityBudget) -> bool {
        let mut state = self.state.lock();
        let valid = state.enabled
            && state.identity_budget == Some(budget)
            && identity.is_some_and(|identity| {
                state.values.finish_pending_identity(identity, budget).is_ok()
            });
        state.completed = valid;
        if !valid {
            if state.enabled {
                state.stats.identity_complete_rejections += 1;
            }
            state.values.abort_revision();
            state.identity_budget = None;
        }
        valid
    }

    /// The explicit transport result, after codegen/link and diagnostics finish.
    /// Missing completion, cancellation, or any failure aborts staging.
    pub fn finish(&self, transport_success: bool) -> bool {
        let mut state = self.state.lock();
        let published = if !transport_success {
            state.stats.publication_transport_failed =
                state.stats.publication_transport_failed.saturating_add(1);
            false
        } else if !state.enabled {
            state.stats.publication_disabled = state.stats.publication_disabled.saturating_add(1);
            false
        } else if !state.completed {
            state.stats.publication_incomplete =
                state.stats.publication_incomplete.saturating_add(1);
            false
        } else {
            match state.values.publish_success() {
                Ok(()) => true,
                Err(error) => {
                    state.stats.publication_storage_failed =
                        state.stats.publication_storage_failed.saturating_add(1);
                    state.stats.last_retention_error = Some(error);
                    false
                }
            }
        };
        if !published {
            state.values.abort_revision();
        }
        if state.enabled {
            if published {
                state.stats.published_revisions += 1;
            } else {
                state.stats.aborted_revisions += 1;
            }
        }
        state.completed = false;
        state.identity_budget = None;
        published
    }

    pub fn get_typeck(
        &self,
        dep_node: DepNode,
        fingerprint: Fingerprint,
    ) -> Option<&'tcx TypeckResults<'tcx>> {
        let mut state = self.state.lock();
        let value = (|| {
            if !state.enabled {
                return None;
            }
            let budget = state.identity_budget?;
            state
                .values
                .typeck_checked(ResidentKey { dep_node, value_fingerprint: fingerprint }, budget)
        })();
        if value.is_some() {
            state.stats.typeck_hits += 1;
        } else {
            state.stats.typeck_misses += 1;
        }
        value
    }

    pub fn get_mir(
        &self,
        dep_node: DepNode,
        fingerprint: Fingerprint,
    ) -> Option<&'tcx mir::Body<'tcx>> {
        let mut state = self.state.lock();
        let value = (|| {
            if !state.enabled {
                return None;
            }
            let budget = state.identity_budget?;
            state
                .values
                .mir_checked(ResidentKey { dep_node, value_fingerprint: fingerprint }, budget)
        })();
        if value.is_some() {
            state.stats.mir_hits += 1;
        } else {
            state.stats.mir_misses += 1;
        }
        value
    }

    /// Called exclusively after the normal dep-graph green validation and
    /// diagnostic replay. The separate leaf experiment permits a source change
    /// only when the selected native body's entire conservative closure remains
    /// current. Inspection can run native item queries, so it occurs outside the
    /// live-state lock; snapshots and the exact body are checked again afterward.
    pub fn get_mir_leaf_after_green(
        &self,
        tcx: TyCtxt<'tcx>,
        dep_node: DepNode,
        fingerprint: Fingerprint,
    ) -> Option<&'tcx mir::Body<'tcx>> {
        if !tcx.resident_live_queries.is_some_and(|state| std::ptr::eq(state, self)) {
            return None;
        }
        let key = ResidentKey { dep_node, value_fingerprint: fingerprint };
        let (body, budget) = {
            let mut state = self.state.lock();
            if !state.enabled || !state.leaf_enabled {
                return None;
            }
            let budget = state.identity_budget?;
            let Some(body) = state.values.mir_leaf_candidate(key, budget) else {
                state.stats.mir_leaf_candidate_misses += 1;
                return None;
            };
            (body, budget)
        };
        let source_map = tcx.sess.source_map();
        let proof = match super::resident_mir_leaf::inspect(
            tcx,
            body,
            source_map,
            super::resident_mir_leaf::LeafLimits::default(),
        ) {
            Ok(proof) => proof,
            Err(reason) => {
                self.state.lock().stats.mir_leaf_last_rejection = Some(reason);
                return None;
            }
        };
        let mut state = self.state.lock();
        if !state.enabled || !state.leaf_enabled || state.identity_budget != Some(budget) {
            return None;
        }
        let value = state.values.mir_leaf_with_proof(key, budget, &proof, source_map)?;
        state.stats.mir_leaf_hits += 1;
        Some(value)
    }

    pub fn insert_typeck(
        &self,
        dep_node: DepNode,
        fingerprint: Fingerprint,
        value: &'tcx TypeckResults<'tcx>,
        estimated_bytes: usize,
    ) -> bool {
        let mut state = self.state.lock();
        let result = if !state.enabled {
            Err(LiveQueryRefusal::Disabled)
        } else if state.identity_budget.is_none() {
            Err(LiveQueryRefusal::IdentityAbsent)
        } else {
            state
                .values
                .retain_typeck(
                    ResidentKey { dep_node, value_fingerprint: fingerprint },
                    value,
                    estimated_bytes,
                )
                .map_err(|error| {
                    state.stats.last_retention_error = Some(error);
                    LiveQueryRefusal::Retention
                })
        };
        state.record_admission(LiveQueryKind::Typeck, result)
    }

    /// Only optimized_mir bodies belong in this namespace.
    pub fn insert_mir(
        &self,
        dep_node: DepNode,
        fingerprint: Fingerprint,
        value: &'tcx mir::Body<'tcx>,
        estimated_bytes: usize,
    ) -> bool {
        let mut state = self.state.lock();
        let result = if !state.enabled {
            Err(LiveQueryRefusal::Disabled)
        } else if state.identity_budget.is_none() {
            Err(LiveQueryRefusal::IdentityAbsent)
        } else {
            state
                .values
                .retain_mir(
                    ResidentKey { dep_node, value_fingerprint: fingerprint },
                    value,
                    estimated_bytes,
                )
                .map_err(|error| {
                    state.stats.last_retention_error = Some(error);
                    LiveQueryRefusal::Retention
                })
        };
        state.record_admission(LiveQueryKind::Mir, result)
    }

    /// For native query hooks that refuse before calling insert. The caller must
    /// report an actually evaluated guard, not infer a cause from a cache miss.
    pub fn record_admission_refusal(&self, kind: LiveQueryKind, reason: LiveQueryRefusal) {
        self.state.lock().record_admission(kind, Err(reason));
    }

    /// Clear telemetry only. Retained values, opt-ins, and revision gates survive.
    pub fn reset_stats(&self) {
        self.state.lock().stats = LiveQueryStats::default();
    }

    pub fn stats(&self) -> LiveQueryStats {
        let state = self.state.lock();
        let (retained_entries, retained_estimated_bytes) = state.values.retained_usage();
        LiveQueryStats { retained_entries, retained_estimated_bytes, ..state.stats }
    }
}

impl LiveState<'_> {
    fn record_admission(
        &mut self,
        kind: LiveQueryKind,
        result: Result<(), LiveQueryRefusal>,
    ) -> bool {
        let (admitted, refusals) = match kind {
            LiveQueryKind::Typeck => {
                (&mut self.stats.typeck_admitted, &mut self.stats.typeck_admission_refusals)
            }
            LiveQueryKind::Mir => {
                (&mut self.stats.mir_admitted, &mut self.stats.mir_admission_refusals)
            }
        };
        match result {
            Ok(()) => *admitted = admitted.saturating_add(1),
            Err(reason) => refusals.record(reason),
        }
        result.is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_accounting_is_separate_saturating_and_resettable() {
        let domain = ResidentDomain::new(LIVE_QUERY_NAMESPACE.to_vec());
        let state = LiveQueryState::new(
            &domain,
            RetentionBudget { max_entries: 1, max_estimated_bytes: 1024 },
        );
        for reason in [
            LiveQueryRefusal::Disabled,
            LiveQueryRefusal::IdentityAbsent,
            LiveQueryRefusal::Dependency,
            LiveQueryRefusal::Diagnostic,
            LiveQueryRefusal::Ineligible,
            LiveQueryRefusal::Retention,
        ] {
            state.record_admission_refusal(LiveQueryKind::Typeck, reason);
        }
        assert_eq!(
            state.stats().typeck_admission_refusals,
            LiveQueryRefusals {
                disabled: 1,
                identity_absent: 1,
                dependency: 1,
                diagnostic: 1,
                ineligible: 1,
                retention: 1,
            }
        );
        assert_eq!(state.stats().mir_admission_refusals, LiveQueryRefusals::default());
        state.state.lock().stats.mir_admission_refusals.dependency = u64::MAX;
        state.record_admission_refusal(LiveQueryKind::Mir, LiveQueryRefusal::Dependency);
        assert_eq!(state.stats().mir_admission_refusals.dependency, u64::MAX);
        assert!(!state.enabled());
        state.enable(true);
        state.reset_stats();
        assert_eq!(state.stats(), LiveQueryStats::default());
        assert!(state.enabled());
    }
}
