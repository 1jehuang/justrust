//! Process-owned snapshots of the graph produced by the preceding revision.
//! Lookup callers must first validate the ordinary incremental header and options.
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use rustc_hashes::Hash64;
use rustc_session::Session;

use super::serialized::SerializedDepGraph;

// 124 MiB total leaves 4 MiB for namespace strings, mutexes and allocator slack.
// Capture includes final grouping arrays and estimated fully built lazy maps.
pub(super) const CAPTURE_LIMIT: usize = 31 * 1024 * 1024;
const CACHE_LIMIT: usize = 62 * 1024 * 1024;
const MAX_NAMESPACES: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Namespace {
    pub(super) root: PathBuf,
    pub(super) options: Hash64,
}

pub(super) fn namespace(sess: &Session) -> Option<Namespace> {
    if !rustc_session::resident::is_process() || !rustc_session::resident::frontend_enabled() {
        return None;
    }
    Some(Namespace {
        root: incremental_root(sess.opts.incremental.as_ref()?)?,
        options: sess.opts.dep_tracking_hash(false),
    })
}

// Missing/deleted incremental directories still identify an unavailable load.
// Canonicalize when possible, otherwise retain an absolute lexical root.
fn incremental_root(root: &std::path::Path) -> Option<PathBuf> {
    std::fs::canonicalize(root).or_else(|_| std::path::absolute(root)).ok()
}

struct Entry {
    namespace: Namespace,
    bytes: Vec<u8>,
    graph: Arc<SerializedDepGraph>,
    cost: usize,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResidentGraphStats {
    pub hits: u64,
    pub misses: u64,
    pub snapshots: u64,
    pub oversized: u64,
    pub evictions: u64,
    pub retained_bytes: usize,
}

struct Cache {
    entries: VecDeque<Entry>,
    stats: ResidentGraphStats,
    capturing: bool,
}

impl Cache {
    const fn new() -> Self {
        Self {
            entries: VecDeque::new(),
            stats: ResidentGraphStats {
                hits: 0,
                misses: 0,
                snapshots: 0,
                oversized: 0,
                evictions: 0,
                retained_bytes: 0,
            },
            capturing: false,
        }
    }

    fn unavailable(&mut self) {
        self.stats.misses += 1;
    }

    fn lookup(&mut self, namespace: &Namespace, bytes: &[u8]) -> Option<Arc<SerializedDepGraph>> {
        let hit = self
            .entries
            .iter()
            .rposition(|entry| &entry.namespace == namespace && entry.bytes == bytes);
        if let Some(index) = hit {
            let entry = self.entries.remove(index).unwrap();
            let graph = entry.graph.clone();
            self.entries.push_back(entry);
            self.stats.hits += 1;
            Some(graph)
        } else {
            self.stats.misses += 1;
            None
        }
    }

    fn publish(
        &mut self,
        namespace: Namespace,
        bytes: Vec<u8>,
        graph: Arc<SerializedDepGraph>,
        graph_cost: usize,
    ) {
        let cost = bytes.capacity().saturating_add(graph_cost);
        if cost > CACHE_LIMIT {
            self.stats.oversized += 1;
            return;
        }
        if let Some(index) = self
            .entries
            .iter()
            .position(|e| e.namespace == namespace && Arc::strong_count(&e.graph) == 1)
        {
            let old = self.entries.remove(index).unwrap();
            self.stats.retained_bytes -= old.cost;
        }
        while self.entries.len() >= MAX_NAMESPACES || self.stats.retained_bytes + cost > CACHE_LIMIT
        {
            // Live sessions keep their graphs charged even while a new snapshot
            // of the same namespace is being constructed.
            let Some(index) = self.entries.iter().position(|e| Arc::strong_count(&e.graph) == 1)
            else {
                return;
            };
            let old = self.entries.remove(index).unwrap();
            self.stats.retained_bytes -= old.cost;
            self.stats.evictions += 1;
        }
        self.entries.push_back(Entry { namespace, bytes, graph, cost });
        self.stats.retained_bytes += cost;
        self.stats.snapshots += 1;
    }
}

static CACHE: Mutex<Cache> = Mutex::new(Cache::new());

pub fn resident_graph_stats() -> ResidentGraphStats {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).stats
}

/// Reuse only a native current-output snapshot whose entire produced file equals
/// `validated_bytes`. This does not replace the loader's format/options validation.
pub fn resident_graph_lookup(
    sess: &Session,
    validated_bytes: &[u8],
) -> Option<Arc<SerializedDepGraph>> {
    let namespace = namespace(sess)?;
    let _lookup_timer = sess.prof.generic_activity("incr_comp_resident_graph_lookup");
    let graph = CACHE.lock().unwrap_or_else(|e| e.into_inner()).lookup(&namespace, validated_bytes);
    let _timer = sess.prof.generic_activity(if graph.is_some() {
        "incr_comp_resident_graph_hit"
    } else {
        "incr_comp_resident_graph_miss"
    });
    graph
}

/// Count one ordinary loader fallback when the graph file is absent or fails
/// format/options validation. This never consults or returns a resident graph.
/// Call exactly once for a non-Ok graph load, not in addition to a lookup miss.
pub fn resident_graph_unavailable(sess: &Session) {
    if namespace(sess).is_none() {
        return;
    }
    let _timer = sess.prof.generic_activity("incr_comp_resident_graph_unavailable");
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).unavailable();
}

// Only one capture can consume the transient half of the process budget.
pub(super) struct CapturePermit;
impl CapturePermit {
    pub(super) fn acquire() -> Option<Self> {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if cache.capturing {
            return None;
        }
        cache.capturing = true;
        Some(Self)
    }
}
impl Drop for CapturePermit {
    fn drop(&mut self) {
        CACHE.lock().unwrap_or_else(|e| e.into_inner()).capturing = false;
    }
}

pub(super) fn oversized() {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).stats.oversized += 1;
}

pub(super) fn publish(
    namespace: Namespace,
    bytes: Vec<u8>,
    graph: Arc<SerializedDepGraph>,
    graph_cost: usize,
) {
    CACHE.lock().unwrap_or_else(|e| e.into_inner()).publish(namespace, bytes, graph, graph_cost);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn namespace(root: &str, options: u64) -> Namespace {
        Namespace { root: root.into(), options: Hash64::new(options) }
    }

    #[test]
    fn exact_bytes_options_root_and_current_revision() {
        let mut cache = Cache::new();
        let ns = namespace("crate-a", 1);
        cache.publish(ns.clone(), vec![1, 2, 3], Arc::default(), 100);
        assert!(cache.lookup(&ns, &[1, 2, 3]).is_some());
        assert!(cache.lookup(&ns, &[1, 2, 4]).is_none());
        assert!(cache.lookup(&ns, &[1, 2]).is_none());
        assert!(cache.lookup(&namespace("crate-a", 2), &[1, 2, 3]).is_none());
        assert!(cache.lookup(&namespace("crate-b", 1), &[1, 2, 3]).is_none());
        // The next revision's produced session_count differs, so replace the
        // preceding output instead of memoizing that revision's input file.
        cache.publish(ns.clone(), vec![1, 2, 4], Arc::default(), 100);
        assert!(cache.lookup(&ns, &[1, 2, 3]).is_none());
        assert!(cache.lookup(&ns, &[1, 2, 4]).is_some());
        assert_eq!(cache.stats.snapshots, 2);
    }

    #[test]
    fn unavailable_graph_counts_fallback_without_bypassing_exact_lookup() {
        let mut cache = Cache::new();
        let ns = namespace("crate-unavailable", 1);
        cache.publish(ns.clone(), vec![1, 2, 3], Arc::default(), 100);
        let bytes = cache.stats.retained_bytes;
        cache.unavailable(); // absent artifact
        cache.unavailable(); // incompatible header/options artifact
        assert_eq!(cache.stats.misses, 2);
        assert_eq!(cache.stats.hits, 0);
        assert_eq!(cache.stats.snapshots, 1);
        assert_eq!(cache.stats.retained_bytes, bytes);
        assert!(cache.lookup(&ns, &[1, 2, 4]).is_none());
        assert_eq!(cache.stats.misses, 3);
        assert!(cache.lookup(&ns, &[1, 2, 3]).is_some());
        assert_eq!(cache.stats.hits, 1);
    }

    #[test]
    fn deleted_incremental_root_still_scopes_unavailable_graph() {
        let root = std::env::current_dir()
            .unwrap()
            .join("resident-test-deleted-root")
            .join("absent-incremental");
        assert_eq!(incremental_root(&root), Some(root));
    }

    #[test]
    fn bounded_lru_eviction_and_pinned_graphs() {
        let mut cache = Cache::new();
        for n in 0..4 {
            cache.publish(namespace("crate", n), vec![n as u8], Arc::default(), 100);
        }
        let pinned = cache.lookup(&namespace("crate", 0), &[0]).unwrap();
        cache.publish(namespace("crate", 4), vec![4], Arc::default(), 100);
        assert!(cache.lookup(&namespace("crate", 1), &[1]).is_none());
        assert!(Arc::ptr_eq(&pinned, &cache.lookup(&namespace("crate", 0), &[0]).unwrap()));
        assert_eq!(cache.entries.len(), 4);
        assert_eq!(cache.stats.evictions, 1);
        let snapshots = cache.stats.snapshots;
        cache.publish(namespace("huge", 0), vec![], Arc::default(), CACHE_LIMIT + 1);
        assert_eq!(cache.stats.snapshots, snapshots);
        assert_eq!(cache.stats.oversized, 1);
        assert!(cache.stats.retained_bytes <= CACHE_LIMIT);
    }
}
