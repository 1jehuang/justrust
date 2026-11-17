//! This module contains `StableHash` implementations for various data types
//! from `rustc_middle::ty` in no particular order.

use std::cell::RefCell;
use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};

use rustc_data_structures::fingerprint::Fingerprint;
use rustc_data_structures::fx::FxHashMap;
use rustc_data_structures::stable_hash::{
    StableHash, StableHashControls, StableHashCtxt, StableHasher,
};
use tracing::trace;

use crate::{mir, ty};

pub(super) fn fresh_hash_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.try_update(Ordering::Relaxed, Ordering::Relaxed, |n| n.checked_add(1))
        .expect("stable hash context generation exhausted")
}

pub(super) fn hash_generation() -> u64 {
    ty::tls::with_opt(|tcx| tcx.map_or(0, |tcx| tcx.stable_hash_generation))
}

/// Arena addresses and crate numbers are only identities inside one GCX.
/// Lazily clear each worker's memo table when its context changes. Generation
/// zero means there is no current GCX, so no previous entry may be reused.
pub(super) fn cache_for_generation<K: Eq + std::hash::Hash, V>(
    cache: &mut (u64, FxHashMap<K, V>),
    generation: u64,
) -> &mut FxHashMap<K, V> {
    if generation == 0 || cache.0 != generation {
        cache.1.clear();
        cache.0 = generation;
    }
    &mut cache.1
}

impl<'tcx, H, T> StableHash for &'tcx ty::list::RawList<H, T>
where
    T: StableHash,
{
    fn stable_hash<Hcx: StableHashCtxt>(&self, hcx: &mut Hcx, hasher: &mut StableHasher) {
        // Note: this cache makes an *enormous* performance difference on certain benchmarks. E.g.
        // without it, compiling `diesel-2.2.10` can be 74% slower, and compiling
        // `deeply-nested-multi` can be ~4,000x slower(!)
        thread_local! {
            static CACHE: RefCell<(u64, FxHashMap<(*const (), StableHashControls), Fingerprint>)> =
                RefCell::new(Default::default());
        }

        let hash = CACHE.with(|cache| {
            let generation = hash_generation();
            let key = (ptr::from_ref(*self).cast::<()>(), hcx.stable_hash_controls());
            if let Some(&hash) = cache_for_generation(&mut cache.borrow_mut(), generation).get(&key)
            {
                return hash;
            }

            let mut hasher = StableHasher::new();
            self[..].stable_hash(hcx, &mut hasher);

            let hash: Fingerprint = hasher.finish();
            cache_for_generation(&mut cache.borrow_mut(), generation).insert(key, hash);
            hash
        });

        hash.stable_hash(hcx, hasher);
    }
}

#[cfg(test)]
mod resident_hash_tests {
    use super::*;

    #[test]
    fn reused_arena_address_cannot_reuse_previous_context_hash() {
        let mut cache = (0, FxHashMap::default());
        let first = fresh_hash_generation();
        let second = fresh_hash_generation();
        cache_for_generation(&mut cache, first).insert(123usize, 456usize);
        assert_eq!(cache_for_generation(&mut cache, first).get(&123), Some(&456));
        assert!(cache_for_generation(&mut cache, second).get(&123).is_none());
        cache_for_generation(&mut cache, second).insert(123, 789);
        assert!(cache_for_generation(&mut cache, first).get(&123).is_none());
    }

    #[test]
    fn hashing_without_a_context_never_reuses_pointer_entries() {
        let mut cache = (0, FxHashMap::default());
        cache_for_generation(&mut cache, 0).insert(123usize, 456usize);
        assert!(cache_for_generation(&mut cache, 0).get(&123).is_none());
    }
}

impl<'tcx> StableHash for ty::GenericArg<'tcx> {
    fn stable_hash<Hcx: StableHashCtxt>(&self, hcx: &mut Hcx, hasher: &mut StableHasher) {
        self.kind().stable_hash(hcx, hasher);
    }
}

// AllocIds get resolved to whatever they point to (to be stable)
impl StableHash for mir::interpret::AllocId {
    fn stable_hash<Hcx: StableHashCtxt>(&self, hcx: &mut Hcx, hasher: &mut StableHasher) {
        ty::tls::with_opt(|tcx| {
            trace!("hashing {:?}", *self);
            let tcx = tcx.expect("can't hash AllocIds during hir lowering");
            tcx.try_get_global_alloc(*self).stable_hash(hcx, hasher);
        });
    }
}

impl StableHash for mir::interpret::CtfeProvenance {
    fn stable_hash<Hcx: StableHashCtxt>(&self, hcx: &mut Hcx, hasher: &mut StableHasher) {
        self.into_parts().stable_hash(hcx, hasher);
    }
}
