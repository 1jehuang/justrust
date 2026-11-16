//! Native, owner-scoped query values for the explicit local same-source pipeline.
//!
//! Nothing starts implicitly. Completed HIR identity must be supplied before
//! analysis, completed identity after analysis, and transport success only after
//! codegen/link and diagnostic finalization. No runtime hits are claimed here.

use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::sync::Lock;

use super::TypeckResults;
use super::resident_identity::{IdentityBudget, RevisionIdentity};
use super::resident_values::{ResidentDomain, ResidentKey, ResidentValues, RetentionBudget};
use crate::dep_graph::DepNode;
use crate::mir;

/// Exact namespace of the factory-owned, controlled local-only first milestone.
pub const LIVE_QUERY_NAMESPACE: &[u8] = b"justrust-local-same-source-v1";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LiveQueryStats {
    pub typeck_hits: u64,
    pub typeck_misses: u64,
    pub mir_hits: u64,
    pub mir_misses: u64,
    pub retained_entries: usize,
    pub retained_estimated_bytes: usize,
}

struct LiveState<'tcx> {
    enabled: bool,
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
        let Some(identity) = identity else { return false };
        if state.values.begin_revision(generation, identity, budget).is_err() {
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
            state.values.abort_revision();
            state.identity_budget = None;
        }
        valid
    }

    /// The explicit transport result, after codegen/link and diagnostics finish.
    /// Missing completion, cancellation, or any failure aborts staging.
    pub fn finish(&self, transport_success: bool) -> bool {
        let mut state = self.state.lock();
        let published = transport_success
            && state.enabled
            && state.completed
            && state.values.publish_success().is_ok();
        if !published {
            state.values.abort_revision();
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
            let (old, next) = state.values.revision_identities()?;
            let authorization = old.authorize_reuse(next, budget).ok()?;
            state
                .values
                .typeck(ResidentKey { dep_node, value_fingerprint: fingerprint }, &authorization)
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
            let (old, next) = state.values.revision_identities()?;
            let authorization = old.authorize_reuse(next, budget).ok()?;
            state
                .values
                .mir(ResidentKey { dep_node, value_fingerprint: fingerprint }, &authorization)
        })();
        if value.is_some() {
            state.stats.mir_hits += 1;
        } else {
            state.stats.mir_misses += 1;
        }
        value
    }

    pub fn insert_typeck(
        &self,
        dep_node: DepNode,
        fingerprint: Fingerprint,
        value: &'tcx TypeckResults<'tcx>,
        estimated_bytes: usize,
    ) -> bool {
        let mut state = self.state.lock();
        state.enabled
            && state.identity_budget.is_some()
            && state
                .values
                .retain_typeck(
                    ResidentKey { dep_node, value_fingerprint: fingerprint },
                    value,
                    estimated_bytes,
                )
                .is_ok()
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
        state.enabled
            && state.identity_budget.is_some()
            && state
                .values
                .retain_mir(
                    ResidentKey { dep_node, value_fingerprint: fingerprint },
                    value,
                    estimated_bytes,
                )
                .is_ok()
    }

    pub fn stats(&self) -> LiveQueryStats {
        let state = self.state.lock();
        let (retained_entries, retained_estimated_bytes) = state.values.retained_usage();
        LiveQueryStats { retained_entries, retained_estimated_bytes, ..state.stats }
    }
}
