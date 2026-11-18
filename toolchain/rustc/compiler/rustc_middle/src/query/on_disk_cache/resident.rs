//! Cross-session encoded archives only. Nothing here owns a tcx, arena, decoded
//! span, source file, allocation, or hygiene decoding context.
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

use super::*;

pub(super) const MAX_BYTES: usize = 128 * 1024 * 1024;
const MAX_NAMESPACES: usize = 4;
const MAX_ROOT: usize = 4096;

#[derive(Clone)]
pub(super) struct Indexes {
    pub file_index_to_stable_id: Arc<FxHashMap<SourceFileIndex, EncodedSourceFileId>>,
    pub query_values_index: Arc<FxHashMap<SerializedDepNodeIndex, AbsoluteBytePos>>,
    pub side_effects_index: Arc<FxHashMap<SerializedDepNodeIndex, AbsoluteBytePos>>,
    pub interpret_alloc_index: Vec<u64>,
    pub syntax_contexts: Arc<FxHashMap<u32, AbsoluteBytePos>>,
    pub expn_data: Arc<UnhashMap<ExpnHash, AbsoluteBytePos>>,
    pub foreign_expn_data: Arc<UnhashMap<ExpnHash, u32>>,
}

impl Indexes {
    pub(super) fn new(footer: Footer) -> Self {
        Self {
            file_index_to_stable_id: Arc::new(footer.file_index_to_stable_id),
            query_values_index: Arc::new(footer.query_values_index.into_iter().collect()),
            side_effects_index: Arc::new(footer.side_effects_index.into_iter().collect()),
            interpret_alloc_index: footer.interpret_alloc_index,
            syntax_contexts: Arc::new(footer.syntax_contexts),
            expn_data: Arc::new(footer.expn_data),
            foreign_expn_data: Arc::new(footer.foreign_expn_data),
        }
    }

    fn retained_bytes(&self) -> usize {
        // Hash tables need buckets, control bytes and allocation headers. Charge
        // conservatively rather than counting only their encoded file footprint.
        fn map<K, V>(capacity: usize) -> usize {
            2 * capacity * (std::mem::size_of::<(K, V)>() + std::mem::size_of::<usize>()) + 64
        }
        fn fx_map<K, V>(map_value: &FxHashMap<K, V>) -> usize {
            map::<K, V>(map_value.capacity())
        }
        fn unhash_map<K, V>(map_value: &UnhashMap<K, V>) -> usize {
            map::<K, V>(map_value.capacity())
        }
        fx_map(&self.file_index_to_stable_id)
            + fx_map(&self.query_values_index)
            + fx_map(&self.side_effects_index)
            + fx_map(&self.syntax_contexts)
            + unhash_map(&self.expn_data)
            + unhash_map(&self.foreign_expn_data)
            + self.interpret_alloc_index.capacity() * std::mem::size_of::<u64>()
            + std::mem::size_of::<Self>()
    }
}

pub(super) struct Archive {
    pub bytes: Arc<[u8]>,
    pub indexes: Indexes,
    // Header length is part of identity even when the artifact bytes match.
    start_pos: Option<usize>,
}

impl Archive {
    fn from_snapshot(bytes: Vec<u8>, footer: Footer) -> Self {
        Self {
            bytes: bytes.into(),
            indexes: Indexes::new(footer),
            // Header validation precedes new(); its writer does not need to
            // separately track the file-format header length.
            start_pos: None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Key {
    root: std::path::PathBuf,
    options: String,
}

pub(super) fn key(sess: &Session) -> Option<Key> {
    if !rustc_session::resident::frontend_enabled() {
        return None;
    }
    let root = sess.opts.incremental.as_ref()?;
    if root.as_os_str().len() > MAX_ROOT {
        return None;
    }
    Some(Key { root: root.clone(), options: format!("{:?}", sess.opts.dep_tracking_hash(false)) })
}

#[derive(Clone, Copy, Debug, Default)]
pub struct ResidentStats {
    pub hits: u64,
    pub misses: u64,
    pub current_bytes: u64,
    pub value_loads: u64,
    pub typeck_loads: u64,
    pub mir_loads: u64,
}

impl ResidentStats {
    fn record_query_load(&mut self, name: &str) {
        self.value_loads += 1;
        // QueryVTable::name is the declared query name. `typeck` is a
        // convenience method, while the actual cached query is `typeck_root`.
        if name == "typeck_root" {
            self.typeck_loads += 1;
        }
        if name.contains("mir") {
            self.mir_loads += 1;
        }
    }
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(Key, Entry)>,
    stats: ResidentStats,
}

/// Idle policy acts only on owned encoded archives, never on live tcx state.
#[derive(Clone, Copy, Debug)]
pub enum IdleTier {
    Compressed,
    Cold,
}

/// Retained owner-cache charges, including indexes and allocation overhead.
/// Active entries are hot entries that an idle transition must leave alone.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct TierStats {
    pub hot_entries: u64,
    pub hot_bytes: u64,
    pub compressed_entries: u64,
    pub compressed_bytes: u64,
    pub active_entries: u64,
}

enum Entry {
    Hot(Arc<Archive>),
    Compressed { bytes: Vec<u8>, decoded_len: usize, start_pos: Option<usize>, indexes: Indexes },
}

impl Entry {
    fn active(&self) -> bool {
        match self {
            Self::Hot(archive) => {
                Arc::strong_count(archive) != 1 || Arc::strong_count(&archive.bytes) != 1
            }
            Self::Compressed { .. } => false,
        }
    }

    fn charge(&self, key: &Key) -> u64 {
        let payload = match self {
            Self::Hot(archive) => archive.bytes.len() + archive.indexes.retained_bytes(),
            Self::Compressed { bytes, indexes, .. } => bytes.capacity() + indexes.retained_bytes(),
        };
        (payload
            + key.root.as_os_str().len()
            + key.options.capacity()
            + std::mem::size_of::<Archive>()
            + std::mem::size_of::<Entry>()
            + std::mem::size_of::<Key>()
            + 64) as u64
    }
}

impl Cache {
    fn unavailable(&mut self, key: Option<Key>) {
        // None means this process/session/namespace is not enabled or eligible.
        if key.is_some() {
            self.stats.misses += 1;
        }
    }

    fn charge(key: &Key, archive: &Archive) -> u64 {
        (archive.bytes.len()
            + archive.indexes.retained_bytes()
            + key.root.as_os_str().len()
            + key.options.capacity()
            + std::mem::size_of::<Archive>()
            + std::mem::size_of::<Entry>()
            + std::mem::size_of::<Key>()
            + 64) as u64
    }

    fn lookup(&mut self, key: &Key, bytes: &[u8], start_pos: usize) -> Option<Arc<Archive>> {
        let Some(pos) = self.entries.iter().position(|(k, _)| k == key) else {
            self.stats.misses += 1;
            return None;
        };
        let archive = match &self.entries[pos].1 {
            Entry::Hot(archive)
                if archive.start_pos.is_none_or(|p| p == start_pos) && &*archive.bytes == bytes =>
            {
                if archive.start_pos.is_none() {
                    // A writer snapshot has no header offset until its first
                    // validated read. Bind it on the first successful lookup.
                    Arc::new(Archive {
                        bytes: archive.bytes.clone(),
                        indexes: archive.indexes.clone(),
                        start_pos: Some(start_pos),
                    })
                } else {
                    archive.clone()
                }
            }
            Entry::Compressed { bytes: encoded, decoded_len, start_pos: header, indexes }
                if header.is_none_or(|p| p == start_pos)
                    && *decoded_len == bytes.len()
                    && *decoded_len <= MAX_BYTES =>
            {
                // Untrusted/corrupt compressed data may never cause unbounded
                // allocation. Exact original bytes, not a digest, establish
                // identity before these native indexes can be used again.
                // Like hot writer snapshots, None is trusted encoder
                // provenance, not a bypass of the caller's header validation.
                let decoded =
                    miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(encoded, *decoded_len);
                match decoded {
                    Ok(decoded) if decoded == bytes => Arc::new(Archive {
                        bytes: decoded.into(),
                        indexes: indexes.clone(),
                        start_pos: Some(start_pos),
                    }),
                    _ => {
                        self.stats.misses += 1;
                        return None;
                    }
                }
            }
            _ => {
                self.stats.misses += 1;
                return None;
            }
        };
        // Promotion uses the ordinary bounded admission path. A corrupt entry
        // above is simply a miss and the caller decodes the normal disk file.
        self.insert(key.clone(), archive.clone());
        self.stats.hits += 1;
        Some(archive)
    }

    fn tier_stats(&self) -> TierStats {
        let mut stats = TierStats::default();
        for (key, entry) in &self.entries {
            match entry {
                Entry::Hot(_) => {
                    stats.hot_entries += 1;
                    stats.hot_bytes += entry.charge(key);
                    stats.active_entries += u64::from(entry.active());
                }
                Entry::Compressed { .. } => {
                    stats.compressed_entries += 1;
                    stats.compressed_bytes += entry.charge(key);
                }
            }
        }
        stats
    }

    fn transition_idle(&mut self, tier: IdleTier) -> TierStats {
        self.entries.retain_mut(|(key, entry)| {
            if entry.active() {
                return true;
            }
            if matches!(tier, IdleTier::Cold) {
                return false;
            }
            if let Entry::Hot(archive) = entry {
                let bytes = miniz_oxide::deflate::compress_to_vec_zlib(&archive.bytes, 1);
                let compressed = Entry::Compressed {
                    bytes,
                    decoded_len: archive.bytes.len(),
                    start_pos: archive.start_pos,
                    indexes: archive.indexes.clone(),
                };
                if compressed.charge(key) < entry.charge(key) {
                    *entry = compressed;
                }
            }
            true
        });
        self.stats.current_bytes = self.entries.iter().map(|(k, e)| e.charge(k)).sum();
        self.tier_stats()
    }

    fn insert(&mut self, key: Key, archive: Arc<Archive>) -> bool {
        let charge = Self::charge(&key, &archive);
        if charge > MAX_BYTES as u64 {
            return false;
        }
        // Only retain the latest artifact for a namespace.
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            let (key, old) = self.entries.remove(pos).unwrap();
            self.stats.current_bytes -= old.charge(&key);
        }
        while self.entries.len() >= MAX_NAMESPACES
            || self.stats.current_bytes + charge > MAX_BYTES as u64
        {
            let (key, old) = self.entries.pop_front().unwrap();
            self.stats.current_bytes -= old.charge(&key);
        }
        self.stats.current_bytes += charge;
        self.entries.push_back((key, Entry::Hot(archive)));
        true
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub(super) fn transition_idle(tier: IdleTier) -> TierStats {
    if !rustc_session::resident::is_process() || !rustc_session::resident::frontend_enabled() {
        return TierStats::default();
    }
    cache().lock().map(|mut cache| cache.transition_idle(tier)).unwrap_or_default()
}

pub(super) fn idle(tier: IdleTier) -> TierStats {
    transition_idle(tier)
}

pub(super) fn tier_stats() -> TierStats {
    if !rustc_session::resident::is_process() || !rustc_session::resident::frontend_enabled() {
        return TierStats::default();
    }
    cache().lock().map(|cache| cache.tier_stats()).unwrap_or_default()
}

pub(super) fn lookup(key: &Key, bytes: &[u8], start_pos: usize) -> Option<Arc<Archive>> {
    cache().lock().ok()?.lookup(key, bytes, start_pos)
}

pub(super) fn unavailable(key: Option<Key>) {
    if key.is_none() {
        return;
    }
    if let Ok(mut cache) = cache().lock() {
        cache.unavailable(key);
    }
}

pub(super) fn admit(
    key: Key,
    bytes: &[u8],
    start_pos: usize,
    footer: Footer,
) -> Option<Arc<Archive>> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let archive = Arc::new(Archive {
        bytes: Arc::from(bytes),
        indexes: Indexes::new(footer),
        start_pos: Some(start_pos),
    });
    cache().lock().ok()?.insert(key, archive.clone()).then_some(archive)
}

pub(super) fn publish(key: Key, bytes: Vec<u8>, footer: Footer) {
    // These bytes come directly from the successfully finished encoder, not a
    // reread of its path. A concurrently replaced file cannot pair unrelated
    // bytes with this native footer. The reader still compares every byte.
    if bytes.len() > MAX_BYTES {
        return;
    }
    let archive = Arc::new(Archive::from_snapshot(bytes, footer));
    if let Ok(mut cache) = cache().lock() {
        cache.insert(key, archive);
    }
}

pub(super) fn value_loaded(name: &str) {
    if let Ok(mut cache) = cache().lock() {
        cache.stats.record_query_load(name);
    }
}

pub(super) fn stats() -> ResidentStats {
    if !rustc_session::resident::frontend_enabled() {
        return ResidentStats::default();
    }
    cache().lock().map(|c| c.stats).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: usize) -> Key {
        Key { root: format!("root/{n}").into(), options: "opts".into() }
    }

    fn archive(bytes: &[u8]) -> Arc<Archive> {
        Arc::new(Archive {
            bytes: Arc::from(bytes),
            indexes: Indexes::new(Footer {
                file_index_to_stable_id: Default::default(),
                query_values_index: Vec::new(),
                side_effects_index: Vec::new(),
                interpret_alloc_index: Vec::new(),
                syntax_contexts: Default::default(),
                expn_data: Default::default(),
                foreign_expn_data: Default::default(),
            }),
            start_pos: Some(8),
        })
    }

    fn assert_charge(cache: &Cache) {
        let tiers = cache.tier_stats();
        assert_eq!(cache.stats.current_bytes, tiers.hot_bytes + tiers.compressed_bytes);
        assert!(cache.stats.current_bytes <= MAX_BYTES as u64);
        assert!(cache.entries.len() <= MAX_NAMESPACES);
    }

    #[test]
    fn compressed_roundtrip_retains_indexes_and_exact_identity() {
        let bytes = vec![42; 8192];
        let archive = archive(&bytes);
        let indexes = archive.indexes.query_values_index.clone();
        let mut cache = Cache::default();
        cache.insert(key(0), archive);
        let hot_bytes = cache.stats.current_bytes;
        let tiers = cache.transition_idle(IdleTier::Compressed);
        assert_eq!((tiers.hot_entries, tiers.compressed_entries), (0, 1));
        assert!(tiers.compressed_bytes < hot_bytes);
        assert_charge(&cache);
        let hit = cache.lookup(&key(0), &bytes, 8).unwrap();
        assert_eq!(&*hit.bytes, bytes);
        assert!(Arc::ptr_eq(&hit.indexes.query_values_index, &indexes));
        assert_eq!(hit.start_pos, Some(8));
        assert_eq!(cache.tier_stats().hot_entries, 1);
        assert_eq!(cache.stats.current_bytes, hot_bytes);
        assert_charge(&cache);
    }

    #[test]
    fn compressed_changed_bytes_or_header_miss_without_promotion() {
        let bytes = vec![42; 8192];
        let mut cache = Cache::default();
        cache.insert(key(0), archive(&bytes));
        cache.transition_idle(IdleTier::Compressed);
        let mut changed = bytes.clone();
        changed[4096] ^= 1;
        assert!(cache.lookup(&key(0), &changed, 8).is_none());
        assert!(cache.lookup(&key(0), &bytes, 9).is_none());
        assert!(cache.lookup(&key(0), &bytes[..4096], 8).is_none());
        assert_eq!(cache.tier_stats().compressed_entries, 1);
        assert_eq!(cache.stats.misses, 3);
        assert!(cache.lookup(&key(0), &bytes, 8).is_some());
        assert_charge(&cache);
    }

    #[test]
    fn corrupted_and_overexpanding_streams_fail_open() {
        let bytes = vec![42; 8192];
        let mut cache = Cache::default();
        cache.insert(key(0), archive(&bytes));
        cache.transition_idle(IdleTier::Compressed);
        if let Entry::Compressed { bytes, .. } = &mut cache.entries[0].1 {
            bytes[0] ^= 0xff;
        } else {
            panic!("expected compressed archive");
        }
        assert!(cache.lookup(&key(0), &bytes, 8).is_none());
        // Admission after the miss is exactly the ordinary disk-decoding path.
        assert!(cache.insert(key(0), archive(&bytes)));
        cache.transition_idle(IdleTier::Compressed);
        if let Entry::Compressed { decoded_len, .. } = &mut cache.entries[0].1 {
            *decoded_len = 16;
        }
        assert!(cache.lookup(&key(0), &[42; 16], 8).is_none());
        if let Entry::Compressed { decoded_len, .. } = &mut cache.entries[0].1 {
            *decoded_len = MAX_BYTES + 1;
        }
        assert!(cache.lookup(&key(0), &bytes, 8).is_none());
        assert_eq!(cache.stats.misses, 3);
        assert_eq!(cache.stats.hits, 0);
        assert_charge(&cache);
    }

    #[test]
    fn idle_transitions_preserve_active_archive_and_encoded_byte_references() {
        let bytes = vec![42; 8192];
        let active = archive(&bytes);
        let standalone_bytes = archive(&bytes);
        let held_bytes = standalone_bytes.bytes.clone();
        let mut cache = Cache::default();
        cache.insert(key(0), active.clone());
        cache.insert(key(1), standalone_bytes);
        cache.insert(key(2), archive(&bytes));
        let tiers = cache.transition_idle(IdleTier::Compressed);
        assert_eq!((tiers.hot_entries, tiers.compressed_entries, tiers.active_entries), (2, 1, 2));
        let tiers = cache.transition_idle(IdleTier::Cold);
        assert_eq!((tiers.hot_entries, tiers.compressed_entries, tiers.active_entries), (2, 0, 2));
        assert_eq!(&*active.bytes, bytes);
        assert_eq!(&*held_bytes, bytes);
        drop(active);
        drop(held_bytes);
        assert_eq!(cache.transition_idle(IdleTier::Cold), TierStats::default());
        assert_eq!(cache.stats.current_bytes, 0);
        assert_charge(&cache);
    }

    #[test]
    fn compression_and_promotion_respect_owner_budget() {
        let bytes = vec![42; MAX_BYTES / 2 + 1];
        let mut cache = Cache::default();
        cache.insert(key(0), archive(&bytes));
        cache.transition_idle(IdleTier::Compressed);
        cache.insert(key(1), archive(&bytes));
        cache.transition_idle(IdleTier::Compressed);
        assert_eq!(cache.tier_stats().compressed_entries, 2);
        assert_charge(&cache);
        assert!(cache.lookup(&key(0), &bytes, 8).is_some());
        assert_charge(&cache);
        assert!(cache.lookup(&key(1), &bytes, 8).is_some());
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries[0].0, key(1));
        assert_charge(&cache);
        assert_eq!(cache.transition_idle(IdleTier::Cold), TierStats::default());
    }

    #[test]
    fn unprofitable_compression_stays_hot() {
        let mut cache = Cache::default();
        cache.insert(key(0), archive(b"x"));
        let before = cache.stats.current_bytes;
        let tiers = cache.transition_idle(IdleTier::Compressed);
        assert_eq!((tiers.hot_entries, tiers.compressed_entries), (1, 0));
        assert_eq!(cache.stats.current_bytes, before);
        assert_charge(&cache);
    }

    #[test]
    fn writer_snapshot_compresses_before_read_then_binds_validated_header() {
        let bytes = vec![42; 8192];
        let mut snapshot = archive(&bytes);
        Arc::get_mut(&mut snapshot).unwrap().start_pos = None;
        let mut cache = Cache::default();
        cache.insert(key(0), snapshot);
        let tiers = cache.transition_idle(IdleTier::Compressed);
        assert_eq!(tiers.compressed_entries, 1);
        assert_eq!(cache.transition_idle(IdleTier::Compressed), tiers);
        let mut changed = bytes.clone();
        changed[4096] ^= 1;
        assert!(cache.lookup(&key(0), &changed, 9).is_none());
        // None has exactly the hot writer snapshot semantics: the caller has
        // validated its header, and every artifact byte must still match.
        let hit = cache.lookup(&key(0), &bytes, 8).unwrap();
        assert_eq!(hit.start_pos, Some(8));
        drop(hit);
        assert_eq!(cache.transition_idle(IdleTier::Compressed).compressed_entries, 1);
        assert!(cache.lookup(&key(0), &bytes, 9).is_none());
        assert!(cache.lookup(&key(0), &bytes, 8).is_some());
        assert_charge(&cache);
    }

    #[test]
    fn same_length_different_bytes_and_namespaces_miss() {
        let mut cache = Cache::default();
        cache.insert(key(0), archive(b"abcdefgh"));
        assert!(cache.lookup(&key(0), b"abcdEfgh", 8).is_none());
        assert!(cache.lookup(&key(1), b"abcdefgh", 8).is_none());
        let mut other_opts = key(0);
        other_opts.options.push('!');
        assert!(cache.lookup(&other_opts, b"abcdefgh", 8).is_none());
        assert!(cache.lookup(&key(0), b"abcdefgh", 7).is_none());
        assert!(cache.lookup(&key(0), b"abcdefgh", 8).is_some());
        assert_eq!(cache.stats.hits, 1);
        assert_eq!(cache.stats.misses, 4);
    }

    #[test]
    fn bounded_lru_and_replacement() {
        let mut cache = Cache::default();
        for n in 0..MAX_NAMESPACES {
            cache.insert(key(n), archive(b"abcdefgh"));
        }
        assert!(cache.lookup(&key(0), b"abcdefgh", 8).is_some());
        cache.insert(key(MAX_NAMESPACES), archive(b"abcdefgh"));
        assert!(cache.lookup(&key(1), b"abcdefgh", 8).is_none());
        assert!(cache.lookup(&key(0), b"abcdefgh", 8).is_some());
        cache.insert(key(0), archive(b"short"));
        assert_eq!(cache.entries.len(), MAX_NAMESPACES);
        assert_eq!(
            cache.stats.current_bytes,
            cache.entries.iter().map(|(key, entry)| entry.charge(key)).sum::<u64>()
        );
    }

    #[test]
    fn byte_limit_evicts_and_rejects_oversize() {
        let mut cache = Cache::default();
        cache.insert(key(0), archive(&vec![0; MAX_BYTES / 2 + 1]));
        cache.insert(key(1), archive(&vec![1; MAX_BYTES / 2 + 1]));
        assert_eq!(cache.entries.len(), 1);
        assert_eq!(cache.entries[0].0, key(1));
        cache.insert(key(2), archive(&vec![2; MAX_BYTES + 1]));
        assert_eq!(cache.entries.len(), 1);
        assert!(cache.stats.current_bytes <= MAX_BYTES as u64);
    }

    #[test]
    fn session_close_does_not_evict_and_new_sessions_share_only_encoded_state() {
        fn assert_owned_send_sync<T: Send + Sync + 'static>() {}
        assert_owned_send_sync::<Archive>();
        let mut input = b"abcdefgh".to_vec();
        let archive = archive(&input);
        input.fill(0);
        assert_eq!(&*archive.bytes, b"abcdefgh");
        let mut cache = Cache::default();
        cache.insert(key(0), archive.clone());
        let first = OnDiskCache::from_archive(archive.clone(), false);
        let second = OnDiskCache::from_archive(archive.clone(), true);
        assert!(!first.resident_archive_hit);
        assert!(second.resident_archive_hit);
        assert!(Arc::ptr_eq(&first.query_values_index, &second.query_values_index));
        assert!(first.file_index_to_file.borrow().is_empty());
        assert!(second.file_index_to_file.borrow().is_empty());
        assert!(!std::ptr::eq(&first.hygiene_context, &second.hygiene_context));
        assert!(!std::ptr::eq(&first.alloc_decoding_state, &second.alloc_decoding_state));
        first.close_serialized_data_mmap();
        assert!(first.serialized_data.read().is_none());
        assert!(second.serialized_data.read().is_some());
        assert!(cache.lookup(&key(0), b"abcdefgh", 8).is_some());
    }

    #[test]
    fn disabled_stats_are_zero() {
        if !rustc_session::resident::is_process() || !rustc_session::resident::frontend_enabled() {
            assert_eq!(tier_stats(), TierStats::default());
            assert_eq!(idle(IdleTier::Compressed), TierStats::default());
            assert_eq!(idle(IdleTier::Cold), TierStats::default());
        }
        if !rustc_session::resident::frontend_enabled() {
            let stats = stats();
            assert_eq!(
                (stats.hits, stats.misses, stats.current_bytes, stats.value_loads),
                (0, 0, 0, 0)
            );
            assert_eq!((stats.typeck_loads, stats.mir_loads), (0, 0));
        }
    }

    #[test]
    fn query_names_count_each_success_once() {
        let mut stats = ResidentStats::default();
        stats.record_query_load("typeck_root");
        stats.record_query_load("type_of_opaque_hir_typeck");
        stats.record_query_load("optimized_mir");
        stats.record_query_load("mir_built");
        stats.record_query_load("generics_of");
        assert_eq!(stats.value_loads, 5);
        assert_eq!(stats.typeck_loads, 1);
        assert_eq!(stats.mir_loads, 2);
    }

    #[test]
    fn unavailable_counts_only_eligible_attempts_without_evicting_archives() {
        let mut cache = Cache::default();
        cache.insert(key(0), archive(b"abcdefgh"));
        let bytes = cache.stats.current_bytes;
        cache.unavailable(None);
        assert_eq!(cache.stats.misses, 0);
        cache.unavailable(Some(key(0)));
        assert_eq!(cache.stats.misses, 1);
        assert_eq!(cache.stats.current_bytes, bytes);
        assert_eq!(cache.stats.value_loads, 0);
        assert!(cache.lookup(&key(0), b"abcdefgh", 8).is_some());
        // A corrupt artifact reaching new() already misses in lookup, before
        // decoding can fail. Its caller must not report unavailable again.
        assert!(cache.lookup(&key(0), b"invalid!", 8).is_none());
        assert_eq!(cache.stats.misses, 2);
    }

    #[test]
    fn native_footer_is_paired_with_writer_snapshot_not_replacement_bytes() {
        let template = archive(b"unused");
        let node = SerializedDepNodeIndex::from_u32(7);
        let footer = Footer {
            file_index_to_stable_id: (*template.indexes.file_index_to_stable_id).clone(),
            query_values_index: vec![(node, AbsoluteBytePos(8))],
            side_effects_index: Vec::new(),
            interpret_alloc_index: vec![9],
            syntax_contexts: Default::default(),
            expn_data: Default::default(),
            foreign_expn_data: Default::default(),
        };
        let snapshot = b"header--native-query-value-and-footer".to_vec();
        let replacement = b"header--foreign-data-with-other-table";
        assert_eq!(snapshot.len(), replacement.len());
        let mut cache = Cache::default();
        cache.insert(key(0), Arc::new(Archive::from_snapshot(snapshot.clone(), footer)));
        assert!(cache.lookup(&key(0), replacement, 8).is_none());
        let hit = cache.lookup(&key(0), &snapshot, 8).unwrap();
        assert_eq!(hit.indexes.query_values_index[&node], AbsoluteBytePos(8));
        assert_eq!(hit.indexes.interpret_alloc_index, vec![9]);
        let session = OnDiskCache::from_archive(hit, true);
        assert!(session.resident_archive_hit);
        assert_eq!(cache.stats.hits, 1);
    }
}
