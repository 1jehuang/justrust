//! Passed tests kept in the store, like compiled functions: a test that passed is not run again
//! while nothing it is a function of has changed, on this machine or on any other that pulls
//! the store from a remote (`ra-jit cache pull`, `RA_JIT_REMOTE`). Only a pass is kept as a
//! result; a failure is kept only to tell a flaky test, and never skips one.
//!
//! What a test is a function of is found in two steps, as a build cache finds what a step read.
//! The first key ([`Results::first`]) is what is known before the test runs: its name, the hash
//! of its code with everything it reaches ([`super::plan::REACH`]), the content of the programs
//! cargo hands it (`CARGO_BIN_EXE_<name>`), the environment the sandbox gives it, what the
//! command builds it with (features, profile, target, configuration) and the harness's flags
//! that change how it runs. A test whose code reaches nothing outside it passes under that key
//! alone. One that reaches files, the environment, processes or the network runs in the sandbox
//! with what it read recorded; the paths it read are kept under the first key (a few sets, for a
//! test that reads other paths as what it read differs), and its pass under the first key with
//! what is at each of those paths now: a file's bytes, whether it can be run, a link's text, a
//! directory's names, or nothing. Of a path the test only looked at (`stat`, `access`), what it
//! could see is what is kept: what is there, whether it can be run, a link's text and a file's
//! size, not its bytes or a directory's names.
//!
//! A test that passed and failed under one key is flaky: everything the sandbox lets it see was
//! the same, so the time, randomness or threads decided. It is marked, and runs every time
//! after.

use std::{
    collections::{BTreeMap, HashMap},
    path::Path,
};

use sha2::{Digest as _, Sha256};

use super::sandboxed;
use crate::{
    content::hex,
    sandbox::{Line, Reached},
};

/// What the store's test results are kept under in place of a toolchain: the toolchain is in
/// each test's hash already.
const RESULTS: &str = "test results 1";

/// How many sets of paths a test's first key keeps: each a run of it that read other paths.
const SETS: u8 = 4;

/// Names where results kept by this command come from, in place of what CI says of the run.
pub(crate) const FROM: &str = "RA_JIT_TESTS_FROM";

pub(crate) type Key = [u8; 32];

/// How much of a path a test saw: only what a `stat` says, or what is in it too. A path seen
/// both ways was seen whole.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Seen {
    Looked,
    Whole,
}

impl Seen {
    fn letter(self) -> &'static str {
        match self {
            Seen::Looked => "l",
            Seen::Whole => "r",
        }
    }
}

/// The paths a test read, each with how much of it it saw.
pub(crate) type Reads = BTreeMap<String, Seen>;

/// What the store has of a test.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Found {
    /// It passed under the key, as the run `from` said.
    Passed { from: String },
    /// It passed and failed under the key: it runs.
    Flaky { from: String },
    /// Nothing that stands for a run now, and why when that is known: the first path a run
    /// that passed read that has changed since.
    Missing(Option<String>),
}

/// The store's test results, and what this command found of the paths tests read.
pub(crate) struct Results {
    store: &'static ra_jit_store::Store,
    from: String,
    inputs: HashMap<(String, Seen), String>,
}

impl Results {
    /// The results of the store this command uses, `None` when it uses none
    /// (`RA_JIT_CACHE=off`).
    pub(crate) fn open() -> Option<Results> {
        ra_jit_store::compiles_against(RESULTS);
        let store = ra_jit_store::Store::global_or(super::daemon::store().as_deref())?;
        Some(Results { store, from: from(), inputs: HashMap::new() })
    }

    /// The first key of the test `name`: `identity` is all that is known of it before it runs.
    pub(crate) fn first(identity: &str, name: &str) -> Key {
        keyed(&[RESULTS.as_bytes(), b"first", identity.as_bytes(), name.as_bytes()])
    }

    /// What the store has of the test with the first key `first`, which reaches the world
    /// outside its code when `reaches`.
    pub(crate) fn find(&mut self, first: &Key, reaches: bool) -> Found {
        if !reaches {
            return self.found(&pure(first));
        }
        let (mut flaky, mut why) = (None, None);
        for set in 0..SETS {
            let Some(kept) = self.store.get(&set_key(first, set)) else { break };
            let Some(kept) = read_set(&kept) else { continue };
            let reads: Reads = kept.iter().map(|(path, (seen, _))| (path.clone(), *seen)).collect();
            let second = self.second(first, &reads);
            match self.found(&second) {
                found @ Found::Passed { .. } => return found,
                found @ Found::Flaky { .. } => flaky = Some(found),
                Found::Missing(_) if why.is_none() => {
                    why = kept
                        .iter()
                        .find(|(path, (seen, then))| self.input(path, *seen) != *then)
                        .map(|(path, _)| format!("it read `{path}`, which changed"));
                }
                Found::Missing(_) => {}
            }
        }
        flaky.unwrap_or(Found::Missing(why))
    }

    /// What the store has under the second key `key`: a mark that the test is flaky, with
    /// where that was found, before any pass.
    fn found(&self, key: &Key) -> Found {
        let text = |it: Vec<u8>| String::from_utf8_lossy(&it).into_owned();
        if let Some(from) = self.store.get(&marked(key, b"flaky")).map(text) {
            return Found::Flaky { from };
        }
        match self.store.get(key).map(text) {
            Some(passed) => {
                let from = passed.strip_prefix("passed\n").unwrap_or_default().to_owned();
                Found::Passed { from }
            }
            None => Found::Missing(None),
        }
    }

    /// Keeps what a run of the test with the first key `first` ended with: `reads` are the
    /// paths it read when it reaches the world outside its code, `None` when it reaches none.
    /// Whether it is flaky: it ended otherwise under the same key before.
    pub(crate) fn keep(&mut self, first: &Key, reads: Option<&Reads>, passed: bool) -> bool {
        let key = match reads {
            None => pure(first),
            Some(reads) => self.second(first, reads),
        };
        let other = marked(&key, b"failed");
        let flaky = match passed {
            true => self.store.get(&other).is_some(),
            false => self.store.get(&key).is_some(),
        };
        match (passed, flaky) {
            (_, true) => self.store.insert(&marked(&key, b"flaky"), self.from.as_bytes()),
            (true, false) => self.store.insert(&key, format!("passed\n{}", self.from).as_bytes()),
            (false, false) => self.store.insert(&other, self.from.as_bytes()),
        }
        if let (true, Some(reads)) = (passed, reads) {
            self.keep_set(first, reads);
        }
        flaky
    }

    /// Has a set of `first` be `reads`, unless one is or every set is taken.
    fn keep_set(&mut self, first: &Key, reads: &Reads) {
        for set in 0..SETS {
            let key = set_key(first, set);
            match self.store.get(&key) {
                Some(kept)
                    if read_set(&kept).is_some_and(|it| {
                        it.iter().map(|(path, (seen, _))| (path, seen)).eq(reads.iter())
                    }) =>
                {
                    return;
                }
                Some(_) => {}
                None => {
                    let mut text = String::new();
                    for (path, &seen) in reads {
                        let input = self.input(path, seen);
                        text.push_str(&format!("{input}\t{}\t{path}\n", seen.letter()));
                    }
                    return self.store.insert(&key, text.as_bytes());
                }
            }
        }
    }

    /// The second key of the test with the first key `first` that read `reads`: with what is
    /// at each now, as much as it saw of it.
    fn second(&mut self, first: &Key, reads: &Reads) -> Key {
        let mut hash = Sha256::new();
        field(&mut hash, RESULTS.as_bytes());
        field(&mut hash, b"read");
        field(&mut hash, first);
        for (path, &seen) in reads {
            let input = self.input(path, seen);
            field(&mut hash, path.as_bytes());
            field(&mut hash, seen.letter().as_bytes());
            field(&mut hash, input.as_bytes());
        }
        hash.finalize().into()
    }

    /// What is at `path` now, as much as `seen`, as [`input`] says, by a hash: found once by
    /// a command.
    fn input(&mut self, path: &str, seen: Seen) -> String {
        self.inputs
            .entry((path.to_owned(), seen))
            .or_insert_with(|| {
                let hash = Sha256::digest(input(Path::new(path), seen).as_bytes());
                hex(&hash[..16])
            })
            .clone()
    }

    /// Writes what was kept to the store's files.
    pub(crate) fn flush(&self) {
        self.store.flush();
    }

    /// Seals what was kept, for the next command and a push to a remote to have.
    pub(crate) fn seal(&self) {
        self.store.seal();
    }
}

/// The paths a test of a run in the sandbox read, by the run's `record`: those its thread read
/// and those read where no test was known. `None` when what it read is not all known.
pub(crate) fn reads_of(record: &[Line], name: &str) -> Option<Reads> {
    let its = |it: &&Line| it.test.is_empty() || it.test == name;
    let mut reads = Reads::new();
    for line in record.iter().filter(its) {
        let seen = match line.reached {
            Reached::Incomplete => return None,
            Reached::Read => Seen::Whole,
            Reached::Looked => Seen::Looked,
            Reached::Write | Reached::Net => continue,
        };
        let it = reads.entry(line.what.clone()).or_insert(seen);
        *it = (*it).max(seen);
    }
    Some(reads)
}

/// What stands for the run this command's results come from: what CI says of it, else
/// [`FROM`], else this machine.
fn from() -> String {
    if let Ok(named) = std::env::var(FROM)
        && !named.is_empty()
    {
        return named;
    }
    let var = |name: &str| std::env::var(name).ok().filter(|it| !it.is_empty());
    match (var("GITHUB_SHA"), var("GITHUB_RUN_ID")) {
        (Some(sha), Some(run)) => format!("commit {} (run {run})", &sha[..sha.len().min(10)]),
        (Some(sha), None) => format!("commit {}", &sha[..sha.len().min(10)]),
        _ => "an earlier run on this machine".to_owned(),
    }
}

/// What is at `path` as a test that saw it as `seen` could tell: nothing, or what it is,
/// whether it can be run, a link's text, and what [`sandboxed::digest`] says of what it leads to
/// when it was seen whole, its size when it is a file only looked at.
fn input(path: &Path, seen: Seen) -> String {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return "-".to_owned() };
    let link = match meta.file_type().is_symlink() {
        true => std::fs::read_link(path).map(|it| it.display().to_string()).unwrap_or_default(),
        false => String::new(),
    };
    #[cfg(unix)]
    let runs = std::fs::metadata(path)
        .is_ok_and(|it| std::os::unix::fs::PermissionsExt::mode(&it.permissions()) & 0o111 != 0);
    #[cfg(not(unix))]
    let runs = false;
    let what = match seen {
        Seen::Whole => sandboxed::digest(path),
        Seen::Looked => match std::fs::metadata(path) {
            Ok(it) if it.is_file() => format!("f{}", it.len()),
            Ok(it) if it.is_dir() => "d".to_owned(),
            Ok(_) => "o".to_owned(),
            Err(_) => "-".to_owned(),
        },
    };
    format!("{link}\0{}\0{what}", u8::from(runs))
}

/// What stands for the programs `paths` (an integration test's `CARGO_BIN_EXE_<name>`) in a
/// test's first key: their bytes, read once by a command.
pub(crate) fn programs(paths: &[&Path]) -> String {
    static KNOWN: std::sync::Mutex<BTreeMap<std::path::PathBuf, String>> =
        std::sync::Mutex::new(BTreeMap::new());
    let mut known = KNOWN.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut hash = Sha256::new();
    for path in paths {
        let digest =
            known.entry(path.to_path_buf()).or_insert_with(|| sandboxed::digest(path)).clone();
        field(&mut hash, path.as_os_str().as_encoded_bytes());
        field(&mut hash, digest.as_bytes());
    }
    format!("-{}", hex(&hash.finalize()[..16]))
}

fn pure(first: &Key) -> Key {
    keyed(&[RESULTS.as_bytes(), b"pure", first])
}

fn set_key(first: &Key, set: u8) -> Key {
    keyed(&[RESULTS.as_bytes(), b"set", first, &[set]])
}

fn marked(key: &Key, mark: &[u8]) -> Key {
    keyed(&[RESULTS.as_bytes(), mark, key])
}

/// A set of paths a test read, each with how much it saw and what was at it, as
/// [`Results::keep_set`] wrote it.
fn read_set(bytes: &[u8]) -> Option<BTreeMap<String, (Seen, String)>> {
    let text = std::str::from_utf8(bytes).ok()?;
    let lines = text.lines().map(|it| {
        let mut parts = it.splitn(3, '\t');
        let (input, seen, path) = (parts.next()?, parts.next()?, parts.next()?);
        let seen = match seen {
            "l" => Seen::Looked,
            "r" => Seen::Whole,
            _ => return None,
        };
        Some((path.to_owned(), (seen, input.to_owned())))
    });
    lines.collect()
}

fn keyed(fields: &[&[u8]]) -> Key {
    let mut hash = Sha256::new();
    for it in fields {
        field(&mut hash, it);
    }
    hash.finalize().into()
}

fn field(hash: &mut Sha256, bytes: &[u8]) {
    hash.update((bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn results(dir: &Path) -> Results {
        // A store of its own: what the process's global one would be is the user's.
        ra_jit_store::compiles_against(RESULTS);
        let store = ra_jit_store::Store::open_at(dir).expect("a store");
        Results {
            store: Box::leak(Box::new(store)),
            from: "commit abc (run 1)".to_owned(),
            inputs: HashMap::new(),
        }
    }

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ra-jit-results-{name}-{}", std::process::id()));
        _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_pass_stands_until_what_the_test_read_changes() {
        let dir = scratch("reads");
        let mut results = results(&dir.join("store"));
        let data = dir.join("data.txt");
        std::fs::write(&data, "one").unwrap();
        let first = Results::first("~abc-env", "tests::reads");
        assert_eq!(results.find(&first, true), Found::Missing(None));
        let paths: Reads = [(data.display().to_string(), Seen::Whole)].into();
        assert!(!results.keep(&first, Some(&paths), true));
        results.seal();
        let passed = Found::Passed { from: "commit abc (run 1)".to_owned() };
        assert_eq!(results.find(&first, true), passed);
        // Another command reads the file again.
        results.inputs.clear();
        std::fs::write(&data, "two").unwrap();
        let why = format!("it read `{}`, which changed", data.display());
        assert_eq!(results.find(&first, true), Found::Missing(Some(why)));
        // Run with the file as it is now, it passes too: both stand.
        assert!(!results.keep(&first, Some(&paths), true));
        results.seal();
        results.inputs.clear();
        std::fs::write(&data, "one").unwrap();
        assert_eq!(results.find(&first, true), passed);
        // A file gone is what is there too.
        results.inputs.clear();
        std::fs::remove_file(&data).unwrap();
        assert!(matches!(results.find(&first, true), Found::Missing(Some(_))));
        // Another test, or the same with other code, has nothing.
        assert_eq!(
            results.find(&Results::first("~abd-env", "tests::reads"), true),
            Found::Missing(None)
        );
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_directory_looked_at_stands_as_names_come_and_go_and_one_listed_does_not() {
        let dir = scratch("looked");
        let mut results = results(&dir.join("store"));
        let (looked, listed) = (dir.join("looked"), dir.join("listed"));
        for it in [&looked, &listed] {
            std::fs::create_dir_all(it).unwrap();
        }
        let first = Results::first("~abc-env", "tests::looks");
        let reads: Reads = [(looked.display().to_string(), Seen::Looked)].into();
        assert!(!results.keep(&first, Some(&reads), true));
        let other = Results::first("~abc-env", "tests::lists");
        let listing: Reads = [(listed.display().to_string(), Seen::Whole)].into();
        assert!(!results.keep(&other, Some(&listing), true));
        results.seal();
        results.inputs.clear();
        for it in [&looked, &listed] {
            std::fs::write(it.join("new"), "").unwrap();
        }
        assert!(matches!(results.find(&first, true), Found::Passed { .. }));
        assert!(matches!(results.find(&other, true), Found::Missing(Some(_))));
        // What a stat tells of a file is its size, and that it is one.
        results.inputs.clear();
        std::fs::remove_dir_all(&looked).unwrap();
        std::fs::write(&looked, "").unwrap();
        assert!(matches!(results.find(&first, true), Found::Missing(Some(_))));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_test_that_passed_and_failed_under_one_key_is_flaky() {
        let dir = scratch("flaky");
        let mut results = results(&dir.join("store"));
        let first = Results::first("abc", "tests::racy");
        assert!(!results.keep(&first, None, false));
        results.seal();
        assert_eq!(results.find(&first, false), Found::Missing(None));
        assert!(results.keep(&first, None, true), "it failed under this key before");
        results.seal();
        assert!(matches!(results.find(&first, false), Found::Flaky { .. }));
        // Failing first or passing first, it is the same.
        let other = Results::first("abc", "tests::other");
        assert!(!results.keep(&other, None, true));
        results.seal();
        assert!(results.keep(&other, None, false));
        results.seal();
        assert!(matches!(results.find(&other, false), Found::Flaky { .. }));
        _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_a_test_read_is_its_own_and_what_no_test_read() {
        let line = |reached, test: &str, what: &str| Line {
            reached,
            test: test.to_owned(),
            what: what.to_owned(),
        };
        let record = [
            line(Reached::Read, "", "/etc/ld.so.cache"),
            line(Reached::Read, "a", "/ws/a.txt"),
            line(Reached::Looked, "a", "/ws/dir"),
            line(Reached::Read, "a", "/ws/dir"),
            line(Reached::Looked, "", "/ws/looked"),
            line(Reached::Read, "b", "/ws/b.txt"),
            line(Reached::Write, "a", "/home/u/x"),
            line(Reached::Incomplete, "c", "a program macOS protects"),
        ];
        let a: Reads = [
            ("/etc/ld.so.cache".to_owned(), Seen::Whole),
            ("/ws/a.txt".to_owned(), Seen::Whole),
            ("/ws/dir".to_owned(), Seen::Whole),
            ("/ws/looked".to_owned(), Seen::Looked),
        ]
        .into();
        assert_eq!(reads_of(&record, "a"), Some(a));
        assert_eq!(reads_of(&record, "c"), None);
    }
}
