//! First resident-value slice, not GlobalCtxt residency. Only explicitly selected
//! owned booleans cross sessions. No arena data, erased storage, or query keys do.
use std::hash::Hash;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::fx::FxHashMap;
use rustc_middle::dep_graph::DepNode;
use rustc_middle::ty::TyCtxt;

const CAPACITY: usize = 8192;
const MAX_ROOT_BYTES: usize = 4096;

#[derive(Clone, PartialEq, Eq, Hash)]
struct Key {
    root: PathBuf,
    options: u64,
    node: DepNode,
    value: Fingerprint,
}

struct Store<K> {
    values: FxHashMap<K, bool>,
    hits: u64,
    misses: u64,
    evictions: u64,
}

impl<K: Eq + Hash> Store<K> {
    fn new() -> Self {
        Self {
            values: FxHashMap::default(),
            hits: 0,
            misses: 0,
            evictions: 0,
        }
    }

    fn get(&mut self, key: &K) -> Option<bool> {
        let value = self.values.get(key).copied();
        if value.is_some() {
            self.hits += 1;
        } else {
            self.misses += 1;
        }
        value
    }

    fn insert(&mut self, key: K, value: bool, capacity: usize) {
        if capacity == 0 {
            return;
        }
        if self.values.len() >= capacity && !self.values.contains_key(&key) {
            self.evictions += self.values.len() as u64;
            // A generation reset bounds both allocation and eviction bookkeeping.
            self.values.clear();
        }
        self.values.insert(key, value);
    }
}

static ENABLED: OnceLock<bool> = OnceLock::new();
static STORE: OnceLock<Mutex<Store<Key>>> = OnceLock::new();

pub(crate) fn enabled() -> bool {
    *ENABLED.get_or_init(|| {
        std::env::var_os("JUSTRUST_RUSTC_RESIDENT_VALUES").is_some_and(|v| v == "1")
    })
}

fn key(tcx: TyCtxt<'_>, node: DepNode, value: Fingerprint) -> Option<Key> {
    if !enabled() {
        return None;
    }
    let (root, options) = tcx
        .query_system
        .resident_value_namespace
        .get_or_init(|| {
            let root = tcx.sess.opts.incremental.as_ref()?;
            let root = if root.is_absolute() {
                root.clone()
            } else {
                std::env::current_dir().ok()?.join(root)
            };
            if root.as_os_str().as_encoded_bytes().len() > MAX_ROOT_BYTES {
                return None;
            }
            Some((root, tcx.sess.opts.dep_tracking_hash(false).as_u64()))
        })
        .as_ref()?;
    Some(Key {
        root: root.clone(),
        options: *options,
        node,
        value,
    })
}

pub(crate) fn get(tcx: TyCtxt<'_>, node: DepNode, fingerprint: Fingerprint) -> Option<bool> {
    let key = key(tcx, node, fingerprint)?;
    let value = STORE
        .get_or_init(|| Mutex::new(Store::new()))
        .lock()
        .ok()?
        .get(&key);
    let _timer = tcx.prof.generic_activity(if value.is_some() {
        "jr_resident_value_hit"
    } else {
        "jr_resident_value_miss"
    });
    value
}

pub(crate) fn insert(tcx: TyCtxt<'_>, node: DepNode, fingerprint: Fingerprint, value: bool) {
    let Some(key) = key(tcx, node, fingerprint) else {
        return;
    };
    if let Ok(mut store) = STORE.get_or_init(|| Mutex::new(Store::new())).lock() {
        store.insert(key, value, CAPACITY);
    }
}

/// Process-wide (hits, misses, evicted entries, live entries). No session pointers.
pub fn counters() -> (u64, u64, u64, usize) {
    STORE
        .get()
        .and_then(|store| store.lock().ok())
        .map_or((0, 0, 0, 0), |store| {
            (
                store.hits,
                store.misses,
                store.evictions,
                store.values.len(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::{Key, Store};
    use rustc_data_structures::fingerprint::Fingerprint;
    use rustc_middle::dep_graph::{DepKind, DepNode};

    fn test_key(root: &str, options: u64, node: u64, value: u64) -> Key {
        Key {
            root: root.into(),
            options,
            node: DepNode {
                kind: DepKind::is_dyn_compatible,
                key_fingerprint: Fingerprint::new(node, 0u64).into(),
            },
            value: Fingerprint::new(value, 0u64),
        }
    }

    #[test]
    fn retains_typed_false_and_separates_fingerprints() {
        let mut store = Store::new();
        store.insert(test_key("root-a", 1, 2, 3), false, 8);
        assert_eq!(store.get(&test_key("root-a", 1, 2, 3)), Some(false));
        for key in [
            test_key("root-b", 1, 2, 3),
            test_key("root-a", 9, 2, 3),
            test_key("root-a", 1, 9, 3),
            test_key("root-a", 1, 2, 9),
        ] {
            assert_eq!(store.get(&key), None);
        }
        let mut other_kind = test_key("root-a", 1, 2, 3);
        other_kind.node.kind = DepKind::is_mir_available;
        assert_eq!(store.get(&other_kind), None);
        assert_eq!((store.hits, store.misses), (1, 5));
    }

    #[test]
    fn bounds_retention_and_updates_existing_entries() {
        let mut store = Store::new();
        store.insert(1, true, 2);
        store.insert(2, false, 2);
        store.insert(1, false, 2);
        assert_eq!(store.get(&1), Some(false));
        store.insert(3, true, 2);
        assert_eq!(store.values.len(), 1);
        assert_eq!(store.evictions, 2);
        assert_eq!(store.get(&1), None);
        assert_eq!(store.get(&3), Some(true));
        store.insert(4, true, 0);
        assert_eq!(store.values.len(), 1);
    }
}
