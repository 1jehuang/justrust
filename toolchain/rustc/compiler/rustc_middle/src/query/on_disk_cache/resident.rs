//! Cross-session encoded archives only. Nothing here owns a tcx, arena, decoded
//! span, source file, allocation, or hygiene decoding context.
use std::collections::VecDeque;
use std::io::Read;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

use super::*;

const MAX_BYTES: usize = 128 * 1024 * 1024;
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
    pub fn new(footer: Footer) -> Self {
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
        fn map<K, V>(map: &std::collections::HashMap<K, V, impl std::hash::BuildHasher>) -> usize {
            2 * map.capacity() * (std::mem::size_of::<(K, V)>() + std::mem::size_of::<usize>()) + 64
        }
        map(&self.file_index_to_stable_id)
            + map(&self.query_values_index)
            + map(&self.side_effects_index)
            + map(&self.syntax_contexts)
            + map(&self.expn_data)
            + map(&self.foreign_expn_data)
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
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(Key, Arc<Archive>)>,
    stats: ResidentStats,
}

impl Cache {
    fn charge(key: &Key, archive: &Archive) -> u64 {
        (archive.bytes.len()
            + archive.indexes.retained_bytes()
            + key.root.as_os_str().len()
            + key.options.capacity()
            + std::mem::size_of::<Archive>()
            + std::mem::size_of::<Key>()
            + 64) as u64
    }

    fn lookup(&mut self, key: &Key, bytes: &[u8], start_pos: usize) -> Option<Arc<Archive>> {
        if let Some(pos) = self.entries.iter().position(|(k, a)| {
            k == key && a.start_pos.is_none_or(|p| p == start_pos) && &*a.bytes == bytes
        }) {
            let entry = self.entries.remove(pos).unwrap();
            let archive = entry.1.clone();
            self.entries.push_back(entry);
            self.stats.hits += 1;
            Some(archive)
        } else {
            self.stats.misses += 1;
            None
        }
    }

    fn insert(&mut self, key: Key, archive: Arc<Archive>) -> bool {
        let charge = Self::charge(&key, &archive);
        if charge > MAX_BYTES as u64 {
            return false;
        }
        // Only retain the latest artifact for a namespace.
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == &key) {
            let (key, old) = self.entries.remove(pos).unwrap();
            self.stats.current_bytes -= Self::charge(&key, &old);
        }
        while self.entries.len() >= MAX_NAMESPACES
            || self.stats.current_bytes + charge > MAX_BYTES as u64
        {
            let (key, old) = self.entries.pop_front().unwrap();
            self.stats.current_bytes -= Self::charge(&key, &old);
        }
        self.stats.current_bytes += charge;
        self.entries.push_back((key, archive));
        true
    }
}

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

pub(super) fn lookup(key: &Key, bytes: &[u8], start_pos: usize) -> Option<Arc<Archive>> {
    cache().lock().ok()?.lookup(key, bytes, start_pos)
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

pub(super) fn publish(key: Key, path: &Path, expected_len: usize, footer: Footer) {
    // Bound both the stat and the read, including a file that grows concurrently.
    let Ok(file) = std::fs::File::open(path) else { return };
    let Ok(metadata) = file.metadata() else { return };
    if metadata.len() != expected_len as u64 || expected_len > MAX_BYTES {
        return;
    }
    let mut bytes = Vec::new();
    if file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes).is_err()
        || bytes.len() != expected_len
    {
        return;
    }
    let archive = Arc::new(Archive {
        bytes: bytes.into(),
        indexes: Indexes::new(footer),
        // The writer does not need to know the file-format header length. The
        // reader already validates the entire header before calling new().
        start_pos: None,
    });
    if let Ok(mut cache) = cache().lock() {
        cache.insert(key, archive);
    }
}

pub(super) fn value_loaded() {
    if let Ok(mut cache) = cache().lock() {
        cache.stats.value_loads += 1;
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
            cache.entries.iter().map(|(key, archive)| Cache::charge(key, archive)).sum::<u64>()
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
        let first = OnDiskCache::from_archive(archive.clone());
        let second = OnDiskCache::from_archive(archive.clone());
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
        if !rustc_session::resident::frontend_enabled() {
            let stats = stats();
            assert_eq!(
                (stats.hits, stats.misses, stats.current_bytes, stats.value_loads),
                (0, 0, 0, 0)
            );
        }
    }
}
