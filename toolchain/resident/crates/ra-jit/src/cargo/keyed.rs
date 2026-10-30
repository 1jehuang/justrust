//! Store entries keyed by everything they were made from, for what is made by reading files
//! that are only known once it is made: a build script's or a proc macro's build, which reads
//! the files of its crates' modules, and a build script's run, which reads what the sandbox
//! records ([#105]).
//!
//! An entry is found in two steps. The first key is of what is known before: the
//! description of a build, or a script, its arguments and its environment. Under it are read
//! sets, each in a slot of its own ([`SLOTS`]): the paths a making of it read. The second key
//! is of the first, a read set and what each of its paths has now ([`digest`]); the entry is
//! under it. An entry is so found only when what it was made of, all of it, is as it was. The
//! store keeps the first value under a key ([`ra_jit_store::Store::insert`]), so a slot is not
//! changed: a making that read another set of paths takes the next free one.
//!
//! [#105]: https://github.com/indexable-inc/blaze/issues/105

use std::path::Path;

use sha2::{Digest as _, Sha256};

/// How many read sets one first key may have: past that, what is made is not kept.
const SLOTS: u32 = 16;

/// What stands for a read of the environment variable after it in a read set: no path starts
/// with it.
const VAR: &str = "$";

/// What stands before a path in a read set when only the [`kind`] of file at it counts.
const KIND: &str = "?";

/// The variable that has every entry found made again and compared ([`check`]).
pub(crate) const CHECK: &str = "RA_JIT_BSCACHE";

/// Whether [`CHECK`] is `check`: an entry found is not used but made again, and what it made
/// compared with the entry. A difference is fatal.
pub(crate) fn check() -> bool {
    std::env::var(CHECK).is_ok_and(|it| it == "check")
}

/// The variable naming a directory where each key done is written as its fields, one a line,
/// in `<key>.txt`: what two machines' keys differ in.
const TRACE: &str = "RA_JIT_KEY_TRACE";

/// A key of fields, each its length and its bytes, and them as text when [`TRACE`] is set.
pub(crate) struct Key(Sha256, Option<Vec<u8>>);

impl Key {
    pub(crate) fn new(kind: &str) -> Key {
        let mut key = Key(Sha256::new(), std::env::var_os(TRACE).map(|_| Vec::new()));
        key.field(kind.as_bytes());
        key
    }

    pub(crate) fn field(&mut self, bytes: &[u8]) -> &mut Key {
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
        if let Some(trace) = &mut self.1 {
            trace.extend(bytes.escape_ascii());
            trace.push(b'\n');
        }
        self
    }

    pub(crate) fn done(self) -> [u8; 32] {
        let key: [u8; 32] = self.0.finalize().into();
        if let (Some(trace), Some(dir)) = (self.1, std::env::var_os(TRACE)) {
            let dir = Path::new(&dir);
            _ = std::fs::create_dir_all(dir);
            _ = std::fs::write(dir.join(format!("{}.txt", hex(&key))), trace);
        }
        key
    }
}

/// What is at `path` now, for a second key: as [`super::sandboxed::digest`] says, but for a
/// path in Nix's store, whose contents never change once there: that it is there is enough;
/// and for a [`kind`], what kind of file it is.
pub(crate) fn digest(path: &str) -> String {
    if let Some(path) = path.strip_prefix(KIND) {
        return match std::fs::metadata(path) {
            Ok(meta) if meta.is_dir() => "d".to_owned(),
            Ok(_) => "o".to_owned(),
            Err(_) => "-".to_owned(),
        };
    }
    if let Some(name) = path.strip_prefix(VAR) {
        return match std::env::var_os(name) {
            Some(value) => format!("={}", hex(&Sha256::digest(value.as_encoded_bytes()))),
            None => "-".to_owned(),
        };
    }
    if let Some(rest) = path.strip_prefix("/nix/store/") {
        let top = rest.split('/').next().unwrap_or_default();
        if Path::new("/nix/store").join(top).exists() {
            return "nix".to_owned();
        }
    }
    super::sandboxed::digest(Path::new(path))
}

/// The read of `path` as a read set has it when only what kind of file is there counts: a
/// directory a program looked for or walked through on its way to others, whose names are not
/// what it read.
pub(crate) fn kind(path: &str) -> String {
    format!("{KIND}{path}")
}

/// What a read set has of a read, `path` or its [`kind`], as a path.
pub(crate) fn path_of(read: &str) -> &str {
    read.strip_prefix(KIND).unwrap_or(read)
}

/// The read of the variable `name`, as a read set has it.
pub(crate) fn var(name: &str) -> String {
    format!("{VAR}{name}")
}

/// The identity of the toolchain at `sysroot`: its rustup channel manifest, which names the
/// release and commit, and the names in its `lib`, whose `librustc_driver-<hash>` names its
/// build where there is no manifest (Nix's).
pub(crate) fn toolchain(sysroot: &Path) -> String {
    let manifest = sysroot.join("lib/rustlib/multirust-channel-manifest.toml");
    let manifest = super::sandboxed::digest(&manifest);
    let lib = super::sandboxed::digest(&sysroot.join("lib"));
    format!("{manifest}/{lib}")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|it| format!("{it:02x}")).collect()
}

fn slot(first: &[u8; 32], at: u32) -> [u8; 32] {
    let mut key = Key::new("read set");
    key.field(first).field(&at.to_le_bytes());
    key.done()
}

/// The second key: of `first`, the paths of `reads` and what each has now, as `now` says.
fn second(first: &[u8; 32], reads: &[String], now: &dyn Fn(&str) -> String) -> [u8; 32] {
    let mut key = Key::new("made of");
    key.field(first);
    for path in reads {
        key.field(path.as_bytes()).field(now(path).as_bytes());
    }
    key.done()
}

fn set_bytes(reads: &[String]) -> Vec<u8> {
    reads.join("\n").into_bytes()
}

fn set_of(bytes: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(bytes).ok()?;
    Some(match text.is_empty() {
        true => Vec::new(),
        false => text.split('\n').map(str::to_owned).collect(),
    })
}

/// The entry under `first` whose read set has, each path, what `now` says it has: with the
/// read set. `None` when none does.
pub(crate) fn find(
    store: &ra_jit_store::Store,
    first: &[u8; 32],
    now: &dyn Fn(&str) -> String,
) -> Option<(Vec<u8>, Vec<String>)> {
    for at in 0..SLOTS {
        let reads = set_of(&store.get(&slot(first, at))?)?;
        if let Some(found) = store.get(&second(first, &reads, now)) {
            return Some((found, reads));
        }
    }
    None
}

/// Keeps `value` under `first` and `reads`, a making of it that read them and found in them
/// what `now` says, with `reads` in a slot under `first` unless one has it already. Sorted,
/// each once, `reads` is what [`find`] gives back.
pub(crate) fn keep(
    store: &ra_jit_store::Store,
    first: &[u8; 32],
    reads: &[String],
    now: &dyn Fn(&str) -> String,
    value: &[u8],
) {
    let mut reads = reads.to_vec();
    reads.sort();
    reads.dedup();
    // A path with a line break can't be in a read set: what read it is not kept.
    if reads.iter().any(|it| it.contains('\n')) {
        return;
    }
    let bytes = set_bytes(&reads);
    for at in 0..SLOTS {
        let key = slot(first, at);
        match store.get(&key) {
            Some(kept) if kept == bytes => break,
            Some(_) => continue,
            None => {
                store.insert(&key, &bytes);
                break;
            }
        }
    }
    store.insert(&second(first, &reads, now), value);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_entry_is_found_while_what_it_read_is_as_it_was() {
        let dir = std::env::temp_dir().join(format!("ra-jit-keyed-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        ra_jit_store::compiles_against("keyed tests");
        // What one process keeps another finds: the store it opens seals what the first left.
        let open = || ra_jit_store::Store::open_at(&dir.join("store")).unwrap();
        let first = Key::new("test").done();
        let kept = |reads: &[String], value: &[u8]| {
            let store = open();
            keep(&store, &first, reads, &digest, value);
            store.flush();
        };
        let found = || find(&open(), &first, &digest);
        let file = dir.join("read.rs");
        std::fs::write(&file, "one").unwrap();
        let path = file.to_str().unwrap().to_owned();
        assert!(found().is_none());
        kept(&[path.clone()], b"made of one");
        let (value, reads) = found().unwrap();
        assert_eq!((value.as_slice(), reads), (&b"made of one"[..], vec![path.clone()]));
        std::fs::write(&file, "two").unwrap();
        assert!(found().is_none(), "a file it read changed");
        // Made again of what is there now, under the same read set.
        kept(&[path.clone()], b"made of two");
        assert_eq!(found().unwrap().0, b"made of two");
        // A making that read another file too takes another slot; both are found.
        let other = dir.join("other.rs");
        std::fs::write(&other, "x").unwrap();
        let both = vec![path.clone(), other.to_str().unwrap().to_owned()];
        std::fs::write(&file, "three").unwrap();
        kept(&both, b"made of three and x");
        assert_eq!(found().unwrap().0, b"made of three and x");
        std::fs::write(&file, "two").unwrap();
        assert_eq!(found().unwrap().0, b"made of two");
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_variable_read_is_its_value() {
        let name = format!("RA_JIT_KEYED_TEST_{}", std::process::id());
        assert_eq!(digest(&var(&name)), "-");
        // SAFETY: no other thread of this test reads this variable.
        unsafe { std::env::set_var(&name, "1") };
        let set = digest(&var(&name));
        assert!(set.starts_with('='), "{set}");
        unsafe { std::env::set_var(&name, "2") };
        assert_ne!(digest(&var(&name)), set);
        unsafe { std::env::remove_var(&name) };
    }
}
