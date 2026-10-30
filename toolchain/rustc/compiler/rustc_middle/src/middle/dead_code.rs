use rustc_data_structures::fx::FxIndexSet;
use rustc_hir::def_id::{DefId, LocalDefId, LocalDefIdMap, LocalDefIdSet};
use rustc_macros::{StableHash, TyDecodable, TyEncodable};

/// An ordered graph edge produced while scanning one item. The root's own
/// liveness and graph traversal remain the responsibility of the caller.
#[derive(Clone, Copy, Debug, TyEncodable, TyDecodable, StableHash)]
pub struct DeadCodeWorkItem {
    pub id: LocalDefId,
    pub propagated_allow_expect: bool,
    pub own_allow_expect: bool,
}

/// Context-independent output of a single HIR node scan, apart from the
/// propagated allow/expect context selected by the query name.
#[derive(Debug, TyEncodable, TyDecodable, StableHash)]
pub struct DeadCodeItemUsage {
    pub worklist: Vec<DeadCodeWorkItem>,
    pub live_symbols: LocalDefIdSet,
    pub ignored_derived_traits: LocalDefIdMap<FxIndexSet<DefId>>,
}

/// A single snapshot of dead-code liveness analysis state.
#[derive(Clone, Debug, StableHash)]
pub struct DeadCodeLivenessSnapshot {
    pub live_symbols: LocalDefIdSet,
    /// Maps each ADT to derived traits (for example `Debug` and `Clone`) that should be ignored
    /// when checking for dead code diagnostics.
    pub ignored_derived_traits: LocalDefIdMap<FxIndexSet<DefId>>,
}

/// Dead-code liveness data for both analysis phases.
///
/// `pre_deferred_seeding` is computed before reachable-public and `#[allow(dead_code)]` seeding,
/// and is used for lint `dead_code_pub_in_binary`.
/// `final_result` is the final liveness snapshot used for lint `dead_code`.
#[derive(Clone, Debug, StableHash)]
pub struct DeadCodeLivenessSummary {
    pub pre_deferred_seeding: DeadCodeLivenessSnapshot,
    pub final_result: DeadCodeLivenessSnapshot,
}
