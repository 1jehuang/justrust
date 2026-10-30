//! Expansions kept by everything they were made from, as `crates/ra-jit/src/cargo/keyed.rs`
//! keeps a build script's or a proc macro's build: a first key of what is known before the
//! expansion, slots of read sets under it, and a second key of the first, a read set and what
//! each of its reads finds now. The format is that file's, but for two things that make an
//! entry the same on every machine: a read's path is the package's (`src/schema.json`, `.` for
//! the package itself), not one of the machine's, and the store's keys are
//! [`ra_jit_store::Store::get_lasting`]'s, of no build of ra-jit or host.

use std::path::Path;

use sha2::{Digest as _, Sha256};

/// How many read sets one first key may have: past that, what is made is not kept.
const SLOTS: u32 = 16;

/// What stands before a path in a read set when only the [`kind`] of file at it counts.
const KIND: &str = "?";

/// A key of fields, each its length and its bytes.
pub(crate) struct Key(Sha256);

impl Key {
    pub(crate) fn new(kind: &str) -> Key {
        let mut key = Key(Sha256::new());
        key.field(kind.as_bytes());
        key
    }

    pub(crate) fn field(&mut self, bytes: &[u8]) -> &mut Key {
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
        self
    }

    pub(crate) fn done(self) -> [u8; 32] {
        self.0.finalize().into()
    }
}

pub(crate) fn sha256(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn short(hash: Sha256) -> String {
    hash.finalize().iter().take(16).map(|it| format!("{it:02x}")).collect()
}

/// What a file of `bytes` has, as [`digest_of`] says of it.
pub(crate) fn digest_bytes(bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(b"f");
    hash.update(bytes);
    short(hash)
}

/// What is at `path`: a file's contents, a directory's names, or that nothing is, as
/// `crates/ra-jit/src/cargo/sandboxed.rs` says it.
pub(crate) fn digest_of(path: &Path) -> String {
    let Ok(meta) = std::fs::metadata(path) else { return "-".to_owned() };
    let mut hash = Sha256::new();
    if meta.is_dir() {
        let mut names: Vec<_> = std::fs::read_dir(path)
            .into_iter()
            .flatten()
            .filter_map(|it| it.ok().map(|it| it.file_name()))
            .collect();
        names.sort();
        hash.update(b"d");
        for name in names {
            hash.update(name.as_encoded_bytes());
            hash.update([0]);
        }
    } else if meta.is_file() {
        match std::fs::read(path) {
            Ok(bytes) => return digest_bytes(&bytes),
            Err(_) => {
                hash.update(b"f");
                hash.update(b"unreadable");
            }
        }
    } else {
        hash.update(b"o");
    }
    short(hash)
}

/// The read of `path` as a read set has it when only what kind of file is there counts.
pub(crate) fn kind(path: &str) -> String {
    format!("{KIND}{path}")
}

/// What the read `read` of the package at `root` finds now.
pub(crate) fn now(root: &Path, read: &str) -> String {
    let (only_kind, rel) = match read.strip_prefix(KIND) {
        Some(rel) => (true, rel),
        None => (false, read),
    };
    let path = if rel == "." { root.to_path_buf() } else { root.join(rel) };
    if !only_kind {
        return digest_of(&path);
    }
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_dir() => "d".to_owned(),
        Ok(_) => "o".to_owned(),
        Err(_) => "-".to_owned(),
    }
}

fn slot(first: &[u8; 32], at: u32) -> [u8; 32] {
    let mut key = Key::new("read set");
    key.field(first).field(&at.to_le_bytes());
    key.done()
}

fn second(first: &[u8; 32], reads: &[(String, String)]) -> [u8; 32] {
    let mut key = Key::new("made of");
    key.field(first);
    for (path, found) in reads {
        key.field(path.as_bytes()).field(found.as_bytes());
    }
    key.done()
}

/// The entry under `first` whose read set finds, each read, what `now` says: `None` when
/// none does.
pub(crate) fn find(
    store: &ra_jit_store::Store,
    first: &[u8; 32],
    now: &dyn Fn(&str) -> String,
) -> Option<Vec<u8>> {
    for at in 0..SLOTS {
        let set = store.get_lasting(&slot(first, at))?;
        let text = std::str::from_utf8(&set).ok()?;
        let reads: Vec<(String, String)> = match text.is_empty() {
            true => Vec::new(),
            false => text.split('\n').map(|it| (it.to_owned(), now(it))).collect(),
        };
        if let Some(found) = store.get_lasting(&second(first, &reads)) {
            return Some(found);
        }
    }
    None
}

/// Keeps `value` under `first` and `reads`, each read with what it found.
pub(crate) fn keep(
    store: &ra_jit_store::Store,
    first: &[u8; 32],
    reads: &[(String, String)],
    value: &[u8],
) {
    // A path with a line break can't be in a read set: what read it is not kept.
    if reads.iter().any(|(it, _)| it.contains('\n')) {
        return;
    }
    let bytes = reads.iter().map(|(it, _)| it.as_str()).collect::<Vec<_>>().join("\n");
    for at in 0..SLOTS {
        let key = slot(first, at);
        match store.get_lasting(&key) {
            Some(kept) if kept == bytes.as_bytes() => break,
            Some(_) => continue,
            None => {
                store.insert_lasting(&key, bytes.as_bytes());
                break;
            }
        }
    }
    store.insert_lasting(&second(first, reads), value);
}
