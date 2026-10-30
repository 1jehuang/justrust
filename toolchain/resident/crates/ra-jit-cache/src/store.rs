//! Where results are kept: in memory for the run, and in files for the runs after it.
//!
//! An entry is a file in the directory of its code, named by a hash of the code and the
//! arguments, holding both, so that two keys of one hash are told apart by what is in the
//! file. It is written to a file of its own and renamed into place, so a reader finds a whole
//! entry or none, whoever stops writing when. When the directory holds more than its size, the
//! entries read or written least recently go first: a hit sets the time of its file.
//!
//! `index/` has a file for each function, with the codes it last ran with: when a run calls it
//! with code it did not have, the results of all but the last few codes go at once.

use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use serde::{Serialize, de::DeserializeOwned};

use crate::__private::{Code, decode, encode};

/// What an entry's file begins with; another format is another name.
const MAGIC: &[u8; 16] = b"ra-jit-fn-cache2";

/// The directory of the index, beside the directories of the codes.
const INDEX: &str = "index";

/// The setting `BLAZE_<name>`, else its old name `RA_JIT_<name>`, as blaze itself reads them.
fn setting(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(format!("BLAZE_{name}")).or_else(|| std::env::var_os(format!("RA_JIT_{name}")))
}

/// How many codes of a function its results are kept for: the code it has and the two before
/// it, so that an edit taken back finds what the code before it computed.
const CODES_KEPT: usize = 3;

/// How much is kept on disk unless `BLAZE_FN_CACHE_SIZE` says.
const DEFAULT_SIZE: u64 = 512 << 20;

/// How much the memory of a run holds before it starts over.
const MEMORY_SIZE: usize = 256 << 20;

/// What a call is looked up by.
pub struct Key {
    code: [u64; 2],
    /// The function, for what `BLAZE_FN_CACHE_LOG` prints.
    name: &'static str,
    args: Vec<u8>,
}

/// The key of a call of the function whose code is `code` with `args`, or `None` when the call
/// is to run as written: ra-jit did not compile the caller, refused to cache the function, or
/// caching is off.
pub fn key<A: Serialize>(code: &Code, args: &A) -> Option<Key> {
    let cache = Cache::get()?;
    if !code.known() {
        let why = code.text();
        if !why.is_empty() && cache.lock().refusals.insert(why) {
            eprintln!("blaze: {why}");
        }
        return None;
    }
    let name = code.text();
    let Some(args) = crate::key::encode(args) else {
        cache.log(name, "not cached: the arguments could not be written");
        return None;
    };
    let mut memory = cache.lock();
    let was = memory.sites.insert(name, code.hash);
    // A live edit gave the function other code: what the old code computed is never asked
    // for again.
    if let Some(was) = was
        && was != code.hash
        && let Some(dropped) = memory.results.remove(&was)
    {
        memory.bytes -= dropped.iter().map(|(k, v)| k.len() + v.len()).sum::<usize>();
        drop(memory);
        cache.log(
            name,
            &format!("the code changed: {} results of the old code dropped", dropped.len()),
        );
    } else {
        drop(memory);
    }
    if was != Some(code.hash)
        && let Some(disk) = &cache.disk
    {
        let dropped = disk.ran(name, code.hash);
        if dropped > 0 {
            cache.log(name, &format!("the results of {dropped} older codes dropped from disk"));
        }
    }
    Some(Key { code: code.hash, name, args })
}

/// What `body` returns for `key`: from memory, from disk, or computed and kept.
pub fn call<R: Serialize + DeserializeOwned>(key: Key, body: impl FnOnce() -> R) -> R {
    let Some(cache) = Cache::get() else { return body() };
    let found = cache.lock().results.get(&key.code).and_then(|it| it.get(&key.args)).cloned();
    if let Some(bytes) = found
        && let Some(value) = decode(&bytes)
    {
        cache.log(key.name, "found in memory");
        return value;
    }
    if let Some(disk) = &cache.disk
        && let Some(bytes) = disk.get(&key)
    {
        if let Some(value) = decode(&bytes) {
            cache.log(key.name, "found on disk");
            cache.remember(key, bytes);
            return value;
        }
        disk.remove(&key);
    }
    let value = body();
    let Some(bytes) = encode(&value) else {
        cache.log(key.name, "computed, not kept: the result could not be written");
        return value;
    };
    match &cache.disk {
        Some(disk) => match disk.put(&key, &bytes) {
            Ok(()) => cache.log(key.name, "computed and kept"),
            Err(e) => cache.log(key.name, &format!("computed, kept in memory: {e}")),
        },
        None => cache.log(key.name, "computed, kept in memory"),
    }
    cache.remember(key, bytes);
    value
}

struct Cache {
    disk: Option<Disk>,
    log: bool,
    memory: Mutex<Memory>,
}

#[derive(Default)]
struct Memory {
    /// Results by the code and then the arguments.
    results: HashMap<[u64; 2], HashMap<Vec<u8>, Vec<u8>>>,
    bytes: usize,
    /// The code each function had at its last call.
    sites: HashMap<&'static str, [u64; 2]>,
    /// The reasons already given for not caching a function.
    refusals: HashSet<&'static str>,
}

impl Cache {
    fn get() -> Option<&'static Cache> {
        static CACHE: OnceLock<Option<Cache>> = OnceLock::new();
        CACHE.get_or_init(Cache::new).as_ref()
    }

    fn new() -> Option<Cache> {
        let mode = setting("FN_CACHE").and_then(|it| it.into_string().ok()).unwrap_or_default();
        if matches!(mode.as_str(), "off" | "0" | "false" | "no") {
            return None;
        }
        let disk = (mode != "memory").then(Disk::find).flatten();
        let log = setting("FN_CACHE_LOG").is_some_and(|it| it != "0");
        Some(Cache { disk, log, memory: Mutex::default() })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Memory> {
        self.memory.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn log(&self, name: &str, what: &str) {
        if self.log {
            eprintln!("blaze cache: {name}: {what}");
        }
    }

    fn remember(&self, key: Key, result: Vec<u8>) {
        let mut memory = self.lock();
        let size = key.args.len() + result.len();
        if memory.bytes + size > MEMORY_SIZE {
            memory.results.clear();
            memory.bytes = 0;
        }
        memory.bytes += size;
        memory.results.entry(key.code).or_default().insert(key.args, result);
    }
}

struct Disk {
    dir: PathBuf,
    limit: u64,
    /// What the directory holds, once a write has looked.
    size: Mutex<Option<u64>>,
}

impl Disk {
    /// `BLAZE_FN_CACHE_DIR`, or `ra-jit/fn-cache` in the target directory the program is in:
    /// the first directory above it with cargo's `CACHEDIR.TAG`. A program elsewhere keeps
    /// its results for the run alone.
    fn find() -> Option<Disk> {
        let dir = match setting("FN_CACHE_DIR") {
            Some(dir) => PathBuf::from(dir),
            None => {
                let exe = std::env::current_exe().ok()?;
                let target =
                    exe.ancestors().skip(1).find(|it| it.join("CACHEDIR.TAG").is_file())?;
                target.join("ra-jit").join("fn-cache")
            }
        };
        let limit = setting("FN_CACHE_SIZE")
            .and_then(|it| it.into_string().ok())
            .and_then(|it| parse_size(&it))
            .unwrap_or(DEFAULT_SIZE);
        Some(Disk { dir, limit, size: Mutex::new(None) })
    }

    fn path(&self, key: &Key) -> PathBuf {
        self.dir.join(code_name(key.code)).join(format!("{:032x}", digest(key)))
    }

    /// Notes that `name` runs with `code`, and takes away what it computed with any code but
    /// the last [`CODES_KEPT`] it ran with. Gives how many codes' results went.
    fn ran(&self, name: &str, code: [u64; 2]) -> usize {
        let index = index_path(&self.dir, name);
        let mut codes = match fs::read_to_string(&index) {
            Ok(text) => match read_index(&text) {
                Some((of, codes)) if of == name => codes,
                // Another function of the same digest, or a torn index: neither is this one's
                // to drop the results of.
                _ => return 0,
            },
            Err(_) => Vec::new(),
        };
        let this = code_name(code);
        if codes.last() == Some(&this) {
            return 0;
        }
        codes.retain(|it| *it != this);
        codes.push(this);
        let gone: Vec<String> = codes.drain(..codes.len().saturating_sub(CODES_KEPT)).collect();
        let mut text = format!("{name}\n");
        for code in &codes {
            text.push_str(code);
            text.push('\n');
        }
        if write_whole(&self.dir, &index, text.as_bytes()).is_err() {
            return 0;
        }
        for old in &gone {
            let _ = fs::remove_dir_all(self.dir.join(old));
        }
        if !gone.is_empty() {
            *self.size.lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        gone.len()
    }

    fn get(&self, key: &Key) -> Option<Vec<u8>> {
        let path = self.path(key);
        let bytes = fs::read(&path).ok()?;
        let result = parse(&bytes, key)?.to_vec();
        // Least recently used goes first: this one was used now.
        if let Ok(file) = fs::File::options().write(true).open(&path) {
            let _ = file.set_modified(SystemTime::now());
        }
        Some(result)
    }

    fn remove(&self, key: &Key) {
        let _ = fs::remove_file(self.path(key));
    }

    fn put(&self, key: &Key, result: &[u8]) -> std::io::Result<()> {
        let path = self.path(key);
        let mut bytes = Vec::with_capacity(MAGIC.len() + 24 + key.args.len() + result.len());
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&key.code[0].to_le_bytes());
        bytes.extend_from_slice(&key.code[1].to_le_bytes());
        bytes.extend_from_slice(&(key.args.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&key.args);
        bytes.extend_from_slice(result);
        write_whole(&self.dir, &path, &bytes)?;
        self.grew(bytes.len() as u64);
        Ok(())
    }

    /// Notes that an entry of `added` bytes was written, and makes room when the directory
    /// holds more than its limit.
    fn grew(&self, added: u64) {
        let mut size = self.size.lock().unwrap_or_else(|e| e.into_inner());
        let now = match *size {
            Some(was) => was + added,
            None => entries(&self.dir).iter().map(|it| it.2).sum(),
        };
        *size = Some(if now > self.limit { self.evict() } else { now });
    }

    /// Takes away the entries used least recently until a quarter of the limit is free, and
    /// what an interrupted write left. Gives what is left.
    fn evict(&self) -> u64 {
        let mut all = entries(&self.dir);
        all.sort_by_key(|it| it.1);
        let mut size: u64 = all.iter().map(|it| it.2).sum();
        let keep = self.limit / 4 * 3;
        for (path, _, len) in all {
            if size <= keep {
                break;
            }
            if fs::remove_file(&path).is_ok() {
                size -= len;
                // Gone when it was the last of its code's.
                let _ = path.parent().map(fs::remove_dir);
            }
        }
        let hour_ago = SystemTime::now() - Duration::from_secs(3600);
        for left in fs::read_dir(self.dir.join("tmp")).into_iter().flatten().flatten() {
            let old = left.metadata().and_then(|it| it.modified()).is_ok_and(|it| it < hour_ago);
            if old {
                let _ = fs::remove_file(left.path());
            }
        }
        size
    }
}

/// The result in the bytes of an entry, when the entry is of `key`.
fn parse<'a>(bytes: &'a [u8], key: &Key) -> Option<&'a [u8]> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let (code, rest) = rest.split_at_checked(16)?;
    let (len, rest) = rest.split_at_checked(8)?;
    let len = usize::try_from(u64::from_le_bytes(len.try_into().ok()?)).ok()?;
    let (args, result) = rest.split_at_checked(len)?;
    let same_code =
        code[..8] == key.code[0].to_le_bytes() && code[8..] == key.code[1].to_le_bytes();
    (same_code && args == key.args).then_some(result)
}

/// What the store holds of one function, for `cargo ra-jit cache`.
pub struct Held {
    pub name: String,
    /// The results kept for each code it ran with and their bytes, the newest code first.
    pub codes: Vec<(u64, u64)>,
}

/// What each function holds in the store at `dir`, by name, and the results and bytes no
/// function's index names: of an older format, or of a function whose index went.
pub fn holdings(dir: &Path) -> (Vec<Held>, (u64, u64)) {
    let indexed = indexes(dir);
    let mut held: Vec<Held> = indexed
        .iter()
        .map(|(name, codes)| Held { name: name.clone(), codes: vec![(0, 0); codes.len()] })
        .collect();
    let at: HashMap<&str, (usize, usize)> = indexed
        .iter()
        .enumerate()
        .flat_map(|(i, (_, codes))| {
            codes.iter().rev().enumerate().map(move |(j, code)| (code.as_str(), (i, j)))
        })
        .collect();
    let mut other = (0, 0);
    for (path, _, len) in entries(dir) {
        let code = path.parent().and_then(Path::file_name).and_then(|it| it.to_str());
        let count = match code.and_then(|it| at.get(it)) {
            Some(&(i, j)) => &mut held[i].codes[j],
            None => &mut other,
        };
        count.0 += 1;
        count.1 += len;
    }
    held.sort_by(|a, b| a.name.cmp(&b.name));
    (held, other)
}

/// Takes away from the store at `dir` the results of every code but each function's newest.
/// Gives the results and bytes that went.
pub fn clear_stale(dir: &Path) -> (u64, u64) {
    let indexed = indexes(dir);
    let newest: HashSet<&str> =
        indexed.iter().filter_map(|(_, codes)| codes.last()).map(String::as_str).collect();
    let mut gone = (0, 0);
    for (path, _, len) in entries(dir) {
        let code = path.parent().and_then(Path::file_name).and_then(|it| it.to_str());
        if code.is_some_and(|it| newest.contains(it)) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            gone.0 += 1;
            gone.1 += len;
            let _ = path.parent().map(fs::remove_dir);
        }
    }
    for (name, codes) in &indexed {
        if let Some(code) = codes.last() {
            let _ =
                write_whole(dir, &index_path(dir, name), format!("{name}\n{code}\n").as_bytes());
        }
    }
    gone
}

/// Each function's index in the store at `dir`: its name and codes, the newest last.
fn indexes(dir: &Path) -> Vec<(String, Vec<String>)> {
    let mut found = Vec::new();
    for file in fs::read_dir(dir.join(INDEX)).into_iter().flatten().flatten() {
        if let Ok(text) = fs::read_to_string(file.path())
            && let Some((name, codes)) = read_index(&text)
        {
            found.push((name.to_owned(), codes));
        }
    }
    found
}

/// `bytes` as the file at `path`, written beside it under `dir` and renamed into place.
fn write_whole(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    static WRITTEN: AtomicU64 = AtomicU64::new(0);
    fs::create_dir_all(path.parent().expect("a file of the store is in a directory"))?;
    let temporary = dir.join("tmp");
    fs::create_dir_all(&temporary)?;
    let n = WRITTEN.fetch_add(1, Ordering::Relaxed);
    let temporary = temporary.join(format!("{}.{n}", std::process::id()));
    let written = fs::File::create(&temporary)
        .and_then(|mut file| file.write_all(bytes))
        .and_then(|()| fs::rename(&temporary, path));
    if written.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    written
}

/// The name of the directory of a code's entries, and of the code in the index.
fn code_name(code: [u64; 2]) -> String {
    format!("{:016x}{:016x}", code[0], code[1])
}

fn index_path(dir: &Path, name: &str) -> PathBuf {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in name.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    dir.join(INDEX).join(format!("{hash:016x}"))
}

/// The function an index file is of and its codes, the newest last.
fn read_index(text: &str) -> Option<(&str, Vec<String>)> {
    let (name, codes) = text.strip_suffix('\n')?.split_once('\n').unwrap_or((text.trim_end(), ""));
    let codes: Vec<String> = codes.lines().map(str::to_owned).collect();
    let named = |it: &String| it.len() == 32 && it.bytes().all(|b| b.is_ascii_hexdigit());
    codes.iter().all(named).then_some((name, codes))
}

/// Every entry under `dir`, with when it was last used and its size.
fn entries(dir: &Path) -> Vec<(PathBuf, SystemTime, u64)> {
    let mut found = Vec::new();
    for shard in fs::read_dir(dir).into_iter().flatten().flatten() {
        if shard.file_name() == "tmp" || shard.file_name() == INDEX {
            continue;
        }
        for entry in fs::read_dir(shard.path()).into_iter().flatten().flatten() {
            if let Ok(meta) = entry.metadata()
                && meta.is_file()
            {
                let used = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                found.push((entry.path(), used, meta.len()));
            }
        }
    }
    found
}

/// FNV-1a of 128 bits over the code and the arguments: the same in every build, and only a
/// name, as an entry holds what it is of.
fn digest(key: &Key) -> u128 {
    const PRIME: u128 = 0x0000000001000000000000000000013B;
    let mut hash: u128 = 0x6c62272e07bb014262b821756295c58d;
    let code = key.code.iter().flat_map(|it| it.to_le_bytes());
    for byte in code.chain(key.args.iter().copied()) {
        hash ^= u128::from(byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn parse_size(text: &str) -> Option<u64> {
    let text = text.trim();
    let (number, shift) = match text.as_bytes().last()? {
        b'K' | b'k' => (&text[..text.len() - 1], 10),
        b'M' | b'm' => (&text[..text.len() - 1], 20),
        b'G' | b'g' => (&text[..text.len() - 1], 30),
        _ => (text, 0),
    };
    number.trim().parse::<u64>().ok()?.checked_mul(1 << shift)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(args: &[u8]) -> Key {
        Key { code: [1, 2], name: "f", args: args.to_vec() }
    }

    fn disk(limit: u64) -> (Disk, PathBuf) {
        static N: AtomicU64 = AtomicU64::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("ra-jit-fn-cache-test-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        (Disk { dir: dir.clone(), limit, size: Mutex::new(None) }, dir)
    }

    #[test]
    fn an_entry_is_found_by_its_key_alone() {
        let (disk, dir) = disk(1 << 20);
        disk.put(&key(b"a"), b"result").unwrap();
        assert_eq!(disk.get(&key(b"a")).as_deref(), Some(&b"result"[..]));
        assert_eq!(disk.get(&key(b"b")), None);
        let other = Key { code: [1, 3], ..key(b"a") };
        assert_eq!(disk.get(&other), None);
        // An entry under the name of another key is not that key's.
        fs::create_dir_all(disk.path(&key(b"b")).parent().unwrap()).unwrap();
        fs::copy(disk.path(&key(b"a")), disk.path(&key(b"b"))).unwrap();
        assert_eq!(disk.get(&key(b"b")), None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_torn_entry_is_none() {
        let (disk, dir) = disk(1 << 20);
        disk.put(&key(b"a"), b"result").unwrap();
        let path = disk.path(&key(b"a"));
        let bytes = fs::read(&path).unwrap();
        fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
        assert!(disk.get(&key(b"a")).is_none_or(|it| it != b"result"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn the_least_recently_used_go_first() {
        let (disk, dir) = disk(4096);
        let big = vec![7u8; 1000];
        for i in 0..3u8 {
            disk.put(&key(&[i]), &big).unwrap();
            std::thread::sleep(Duration::from_millis(20));
        }
        // The first is used again, so the second is the oldest.
        assert!(disk.get(&key(&[0])).is_some());
        std::thread::sleep(Duration::from_millis(20));
        disk.put(&key(&[3]), &big).unwrap();
        disk.put(&key(&[4]), &big).unwrap();
        assert!(disk.get(&key(&[1])).is_none(), "the oldest went");
        assert!(disk.get(&key(&[4])).is_some(), "the newest stayed");
        assert!(entries(&dir).iter().map(|it| it.2).sum::<u64>() <= 4096);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_function_keeps_the_results_of_its_last_codes_alone() {
        let (disk, dir) = disk(1 << 20);
        let of = |code: u64| Key { code: [code, 0], name: "f", args: vec![1] };
        for code in 1..=3 {
            assert_eq!(disk.ran("f", of(code).code), 0);
            disk.put(&of(code), b"result").unwrap();
        }
        disk.put(&Key { code: [9, 0], name: "g", args: vec![1] }, b"other").unwrap();
        assert_eq!(disk.ran("g", [9, 0]), 0);
        // A fourth code drops the first's results, and only them.
        assert_eq!(disk.ran("f", of(4).code), 1);
        assert!(disk.get(&of(1)).is_none());
        assert!(disk.get(&of(2)).is_some() && disk.get(&of(3)).is_some());
        assert!(disk.get(&Key { code: [9, 0], name: "g", args: vec![1] }).is_some());
        // An edit taken back finds what its code computed, and drops nothing.
        assert_eq!(disk.ran("f", of(2).code), 0);
        assert!(disk.get(&of(2)).is_some());
        assert_eq!(disk.ran("f", of(2).code), 0);
        let (held, other) = holdings(&dir);
        let f = held.iter().find(|it| it.name == "f").unwrap();
        assert_eq!(f.codes, vec![(1, f.codes[0].1), (0, 0), (1, f.codes[2].1)]);
        assert_eq!(other, (0, 0));
        // `clear --stale` keeps each function's newest code.
        assert_eq!(clear_stale(&dir).0, 1);
        assert!(disk.get(&of(2)).is_some() && disk.get(&of(3)).is_none());
        assert!(disk.get(&Key { code: [9, 0], name: "g", args: vec![1] }).is_some());
        assert_eq!(holdings(&dir).0.iter().find(|it| it.name == "f").unwrap().codes.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn an_index_is_read_as_written() {
        let code = code_name([0xab, 0xcd]);
        let text = format!("app::f::<u32>\n{code}\n");
        assert_eq!(read_index(&text), Some(("app::f::<u32>", vec![code.clone()])));
        assert_eq!(read_index("app::f\n"), Some(("app::f", vec![])));
        assert_eq!(read_index(&format!("app::f\n{}", &code[..10])), None, "torn");
    }

    #[test]
    fn sizes() {
        assert_eq!(parse_size("512M"), Some(512 << 20));
        assert_eq!(parse_size("10"), Some(10));
        assert_eq!(parse_size("1g"), Some(1 << 30));
        assert_eq!(parse_size("x"), None);
    }
}
