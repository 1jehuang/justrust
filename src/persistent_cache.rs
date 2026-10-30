//! Best-effort immutable byte outputs for a local, trusted cache directory.
//!
//! The checksum detects accidental corruption, not malicious replacement. Cache
//! keys must describe every input that can affect an output. Publication is
//! atomic, but this cache does not promise durability across power loss.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const MAGIC: &[u8; 8] = b"JRLCACHE";
const VERSION: u32 = 1;
/// Maximum payload size accepted by the local cache.
pub const MAX_ENTRY_BYTES: usize = 64 * 1024 * 1024;
const HEADER_SIZE: usize = 8 + 4 + 32 + 8 + 32;
const KEY_DOMAIN: &[u8] = b"justrust.local-cache.key";

/// A local cache whose read failures are indistinguishable from misses.
#[derive(Clone, Debug)]
pub struct LocalCache {
    directory: PathBuf,
}

impl LocalCache {
    pub fn new(directory: PathBuf) -> Self {
        Self { directory }
    }

    fn entry_path(&self, key: &[u8; 32]) -> PathBuf {
        self.directory
            .join(blake3::Hash::from(*key).to_hex().as_str())
    }

    /// Return a fully validated output, or a miss for any I/O or format error.
    pub fn load(&self, key: &[u8; 32]) -> Option<Vec<u8>> {
        self.load_bounded(key, MAX_ENTRY_BYTES)
    }

    /// Apply a caller's per-operation allocation bound before decoding bytes.
    pub fn load_bounded(&self, key: &[u8; 32], max_bytes: usize) -> Option<Vec<u8>> {
        self.read_entry(key, max_bytes.min(MAX_ENTRY_BYTES)).ok()
    }

    /// An upper bound for a subsequent bounded read, so a graph node can admit
    /// its byte storage before decoding. This is not a validated cache hit.
    /// If a writer replaces the entry after this observation, load_bounded
    /// still enforces the admitted bound and validates the complete record.
    pub fn payload_size(&self, key: &[u8; 32], max_bytes: usize) -> Option<usize> {
        let metadata = std::fs::symlink_metadata(self.entry_path(key)).ok()?;
        if !metadata.is_file() {
            return None;
        }
        let bytes = usize::try_from(metadata.len())
            .ok()?
            .checked_sub(HEADER_SIZE)?;
        (bytes <= max_bytes.min(MAX_ENTRY_BYTES)).then_some(bytes)
    }

    fn read_entry(&self, key: &[u8; 32], max_bytes: usize) -> io::Result<Vec<u8>> {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        // Linux x86-64 UAPI O_NOFOLLOW | O_NONBLOCK. A named pipe must be a
        // miss even with no writer, and an entry must not redirect the read.
        options.custom_flags(0x20000 | 0x800);
        let mut file = options.open(self.entry_path(key))?;
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(invalid("cache entry is not a regular file"));
        }
        let size = metadata.len();
        if !(HEADER_SIZE as u64..=(HEADER_SIZE + max_bytes) as u64).contains(&size) {
            return Err(invalid("cache file size out of bounds"));
        }
        let mut header = [0; HEADER_SIZE];
        file.read_exact(&mut header)?;
        if &header[..8] != MAGIC || header[8..12] != VERSION.to_le_bytes() || &header[12..44] != key
        {
            return Err(invalid("cache header mismatch"));
        }
        let length = u64::from_le_bytes(header[44..52].try_into().unwrap());
        if length > max_bytes as u64 || size != HEADER_SIZE as u64 + length {
            return Err(invalid("cache payload length mismatch"));
        }
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(length as usize)
            .map_err(|_| io::Error::other("cache allocation failed"))?;
        bytes.resize(length as usize, 0);
        file.read_exact(&mut bytes)?;
        let mut trailing = [0];
        if file.read(&mut trailing)? != 0 || blake3::hash(&bytes).as_bytes() != &header[52..84] {
            return Err(invalid("cache payload corruption"));
        }
        Ok(bytes)
    }

    /// Atomically publish an output. Concurrent writers never expose partial data.
    /// Callers are responsible for using a key only for equivalent outputs.
    pub fn store(&self, key: &[u8; 32], bytes: &[u8]) -> io::Result<()> {
        if bytes.len() > MAX_ENTRY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cache entry exceeds 64 MiB",
            ));
        }
        create_directory(&self.directory)?;
        let (mut file, mut temporary) = self.create_temporary()?;
        file.write_all(MAGIC)?;
        file.write_all(&VERSION.to_le_bytes())?;
        file.write_all(key)?;
        file.write_all(&(bytes.len() as u64).to_le_bytes())?;
        file.write_all(blake3::hash(bytes).as_bytes())?;
        file.write_all(bytes)?;
        file.flush()?;
        drop(file);
        fs::rename(&temporary.0, self.entry_path(key))?;
        // Disarm cleanup so another writer can safely reuse the temporary name.
        temporary.1 = true;
        Ok(())
    }

    fn create_temporary(&self) -> io::Result<(File, Temporary)> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for attempt in 0..128 {
            let path = self
                .directory
                .join(format!(".tmp-{}-{nonce}-{attempt}", std::process::id()));
            let mut options = OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
            match options.open(&path) {
                Ok(file) => return Ok((file, Temporary(path, false))),
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "cache temporary name collisions",
        ))
    }
}

fn create_directory(path: &std::path::Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    builder.mode(0o700);
    // DirBuilder leaves existing directories and their permissions unchanged.
    builder.create(path)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

struct Temporary(PathBuf, bool);

impl Drop for Temporary {
    fn drop(&mut self) {
        if !self.1 {
            let _ = fs::remove_file(&self.0);
        }
    }
}

/// Versioned, domain-separated keys with unambiguous length-delimited fields.
pub struct KeyBuilder(blake3::Hasher);

impl KeyBuilder {
    pub fn new(domain: &[u8], compiler: &[u8; 32]) -> Self {
        let mut builder = Self(blake3::Hasher::new());
        builder.field(KEY_DOMAIN);
        builder.field(&VERSION.to_le_bytes());
        builder.field(domain);
        builder.field(compiler);
        builder
    }

    pub fn field(&mut self, bytes: &[u8]) {
        self.0.update(&(bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
    }

    pub fn finish(self) -> [u8; 32] {
        *self.0.finalize().as_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    struct Directory(PathBuf);

    impl Directory {
        fn new() -> Self {
            let scratch = std::env::var_os("JCODE_SCRATCH_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            create_directory(&scratch).unwrap();
            let base = LocalCache::new(scratch);
            let (file, temporary) = base.create_temporary().unwrap();
            drop(file);
            fs::remove_file(&temporary.0).unwrap();
            create_directory(&temporary.0).unwrap();
            Self(temporary.0.clone())
        }

        fn cache(&self) -> LocalCache {
            LocalCache::new(self.0.join("cache"))
        }
    }

    impl Drop for Directory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn key(domain: &[u8], compiler: u8, fields: &[&[u8]]) -> [u8; 32] {
        let mut builder = KeyBuilder::new(domain, &[compiler; 32]);
        for field in fields {
            builder.field(field);
        }
        builder.finish()
    }

    #[test]
    fn keys_are_deterministic_and_framed() {
        let original = key(b"domain", 0, &[b"ab", b"c"]);
        assert_eq!(original, key(b"domain", 0, &[b"ab", b"c"]));
        for distinct in [
            key(b"other", 0, &[b"ab", b"c"]),
            key(b"domain", 1, &[b"ab", b"c"]),
            key(b"domain", 0, &[b"a", b"bc"]),
            key(b"domain", 0, &[b"abc"]),
            key(b"domain", 0, &[b"ab", b"c", b""]),
            key(b"domain", 0, &[b"c", b"ab"]),
        ] {
            assert_ne!(original, distinct);
        }
        assert_ne!(key(b"d", 0, &[]), key(b"d", 0, &[b""]));
        let mut future = KeyBuilder(blake3::Hasher::new());
        future.field(KEY_DOMAIN);
        future.field(&(VERSION + 1).to_le_bytes());
        future.field(b"domain");
        future.field(&[0; 32]);
        future.field(b"ab");
        future.field(b"c");
        assert_ne!(original, future.finish());
    }

    #[test]
    fn round_trip_empty_binary_distinct_and_reopened() {
        let directory = Directory::new();
        let cache = directory.cache();
        assert_eq!(cache.load(&[0; 32]), None);
        cache.store(&[0; 32], b"").unwrap();
        cache.store(&[1; 32], &[0, 255, 17]).unwrap();
        assert_eq!(directory.cache().load(&[0; 32]), Some(vec![]));
        assert_eq!(directory.cache().load(&[1; 32]), Some(vec![0, 255, 17]));
        assert_eq!(cache.load_bounded(&[1; 32], 2), None);
        assert_eq!(cache.load_bounded(&[1; 32], 3), Some(vec![0, 255, 17]));
        assert_eq!(cache.load_bounded(&[0; 32], 0), Some(vec![]));
        assert_eq!(
            cache.load_bounded(&[1; 32], usize::MAX),
            Some(vec![0, 255, 17])
        );
        assert_eq!(cache.load(&[2; 32]), None);
        assert_eq!(fs::read_dir(&cache.directory).unwrap().count(), 2);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn nonregular_cache_entries_are_nonblocking_misses() {
        use std::os::unix::ffi::OsStrExt;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        const CHILD: &str = "JUSTRUST_CACHE_FIFO_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let directory = Directory::new();
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "persistent_cache::tests::nonregular_cache_entries_are_nonblocking_misses",
                    "--nocapture",
                    "--test-threads=1",
                ])
                .env(CHILD, &directory.0)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                if child.try_wait().unwrap().is_some() {
                    let output = child.wait_with_output().unwrap();
                    assert!(output.status.success(), "{output:?}");
                    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
                    return;
                }
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let output = child.wait_with_output().unwrap();
                    panic!("cache read blocked on a nonregular entry: {output:?}");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }

        let cache = LocalCache::new(PathBuf::from(std::env::var_os(CHILD).unwrap()).join("cache"));
        create_directory(&cache.directory).unwrap();
        let path = cache.entry_path(&[7; 32]);
        let fifo = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        unsafe extern "C" {
            fn mkfifo(path: *const std::ffi::c_char, mode: u32) -> i32;
        }
        // Valid terminated pathname, Linux mode_t is u32. No writer is opened,
        // so a blocking O_RDONLY open would never return before the deadline.
        assert_eq!(unsafe { mkfifo(fifo.as_ptr(), 0o600) }, 0);
        assert_eq!(cache.payload_size(&[7; 32], MAX_ENTRY_BYTES), None);
        assert_eq!(cache.load(&[7; 32]), None);
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    #[test]
    fn symlink_entries_cannot_redirect_valid_cache_reads_or_writes() {
        use std::os::unix::fs::symlink;
        let directory = Directory::new();
        let cache = directory.cache();
        let other = LocalCache::new(directory.0.join("other"));
        let key = [9; 32];
        other.store(&key, b"original").unwrap();
        create_directory(&cache.directory).unwrap();
        symlink(other.entry_path(&key), cache.entry_path(&key)).unwrap();
        assert_eq!(cache.payload_size(&key, MAX_ENTRY_BYTES), None);
        assert_eq!(cache.load(&key), None);
        assert_eq!(other.load(&key), Some(b"original".to_vec()));
        cache.store(&key, b"replacement").unwrap();
        assert_eq!(cache.load(&key), Some(b"replacement".to_vec()));
        assert_eq!(other.load(&key), Some(b"original".to_vec()));

        fs::remove_file(cache.entry_path(&key)).unwrap();
        symlink("/dev/zero", cache.entry_path(&key)).unwrap();
        assert_eq!(cache.load(&key), None);
        fs::remove_file(cache.entry_path(&key)).unwrap();
        fs::create_dir(cache.entry_path(&key)).unwrap();
        assert_eq!(cache.load(&key), None);
    }

    #[test]
    fn rejects_every_truncation_and_each_corrupt_byte() {
        let directory = Directory::new();
        let cache = directory.cache();
        let key = [5; 32];
        cache.store(&key, b"payload").unwrap();
        let path = cache.entry_path(&key);
        let valid = fs::read(&path).unwrap();
        for end in 0..valid.len() {
            fs::write(&path, &valid[..end]).unwrap();
            assert_eq!(cache.load(&key), None, "truncation at {end}");
        }
        for index in 0..valid.len() {
            let mut corrupt = valid.clone();
            corrupt[index] ^= 1;
            fs::write(&path, corrupt).unwrap();
            assert_eq!(cache.load(&key), None, "corruption at {index}");
        }
        let mut trailing = valid.clone();
        trailing.push(0);
        fs::write(&path, trailing).unwrap();
        assert_eq!(cache.load(&key), None);
        fs::write(cache.entry_path(&[6; 32]), valid).unwrap();
        assert_eq!(cache.load(&[6; 32]), None);
    }

    #[test]
    fn size_limit_is_inclusive_and_rejects_oversize_before_io() {
        let directory = Directory::new();
        let cache = directory.cache();
        let mut payload = vec![42; MAX_ENTRY_BYTES];
        cache.store(&[0; 32], &payload).unwrap();
        assert_eq!(cache.load(&[0; 32]).unwrap(), payload);
        payload.push(42);
        assert_eq!(
            cache.store(&[1; 32], &payload).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let path = cache.entry_path(&[0; 32]);
        OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len((HEADER_SIZE + MAX_ENTRY_BYTES + 1) as u64)
            .unwrap();
        assert_eq!(cache.load(&[0; 32]), None);
        fs::write(&path, [0; HEADER_SIZE]).unwrap();
        assert_eq!(cache.load(&[0; 32]), None);
    }

    #[test]
    fn io_failures_are_misses_and_failed_publication_cleans_temporary() {
        let directory = Directory::new();
        let obstacle = directory.0.join("file");
        fs::write(&obstacle, b"not a directory").unwrap();
        let cache = LocalCache::new(obstacle);
        assert!(cache.store(&[0; 32], b"data").is_err());
        assert_eq!(cache.load(&[0; 32]), None);
        let cache = directory.cache();
        fs::create_dir_all(cache.entry_path(&[0; 32])).unwrap();
        assert!(cache.store(&[0; 32], b"data").is_err());
        assert_eq!(cache.load(&[0; 32]), None);
        assert_eq!(fs::read_dir(&cache.directory).unwrap().count(), 1);
    }

    #[cfg(unix)]
    #[test]
    fn unix_modes_are_private_and_existing_directories_are_preserved() {
        use std::os::unix::fs::PermissionsExt;

        let directory = Directory::new();
        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        let cache = LocalCache::new(directory.0.join("parent").join("cache"));
        cache.store(&[0; 32], b"private output").unwrap();
        assert_eq!(mode(cache.directory.parent().unwrap()), 0o700);
        assert_eq!(mode(&cache.directory), 0o700);
        assert_eq!(mode(&cache.entry_path(&[0; 32])), 0o600);
        let (file, temporary) = cache.create_temporary().unwrap();
        assert_eq!(mode(&temporary.0), 0o600);
        drop(file);
        drop(temporary);

        fs::set_permissions(&cache.directory, fs::Permissions::from_mode(0o750)).unwrap();
        cache.store(&[1; 32], b"another output").unwrap();
        assert_eq!(mode(&cache.directory), 0o750);
        assert_eq!(mode(&cache.entry_path(&[1; 32])), 0o600);
    }

    #[test]
    fn concurrent_writers_and_readers_observe_only_complete_entries() {
        let directory = Directory::new();
        let cache = Arc::new(directory.cache());
        cache.store(&[0; 32], &[0; 4096]).unwrap();
        let barrier = Arc::new(Barrier::new(9));
        thread::scope(|scope| {
            for writer in 1..=8u8 {
                let cache = Arc::clone(&cache);
                let barrier = Arc::clone(&barrier);
                scope.spawn(move || {
                    barrier.wait();
                    for _ in 0..16 {
                        cache.store(&[0; 32], &[writer; 4096]).unwrap();
                        cache.store(&[writer; 32], &[writer; 4096]).unwrap();
                    }
                });
            }
            barrier.wait();
            for _ in 0..128 {
                let bytes = cache
                    .load(&[0; 32])
                    .expect("atomic replacement must stay readable");
                assert_eq!(bytes.len(), 4096);
                assert!(bytes[0] <= 8);
                assert!(bytes.iter().all(|byte| *byte == bytes[0]));
            }
        });
        for writer in 1..=8u8 {
            assert_eq!(cache.load(&[writer; 32]), Some(vec![writer; 4096]));
        }
        assert_eq!(fs::read_dir(&cache.directory).unwrap().count(), 9);
    }
}
