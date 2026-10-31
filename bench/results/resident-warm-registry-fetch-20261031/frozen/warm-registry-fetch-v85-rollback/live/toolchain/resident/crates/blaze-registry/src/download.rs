//! A package's `.crate` file: found in blaze's store or cargo's cache, or downloaded by the
//! registry's `dl` template; checked against its sha256; kept in the store by that hash and in
//! cargo's cache; and unpacked where cargo unpacks it, with cargo 1.99's checks
//! (`src/sources/registry/{download,mod}.rs`).

use std::{
    fs::OpenOptions,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

use anyhow::{Context as _, bail};
use semver::Version;
use sha2::{Digest, Sha256};
use tar::{Archive, EntryType};

use crate::{http::HttpNotSuccessful, index::dep_path, registry::Registry};

/// Marks a package unpacked whole. Not the `.cargo-ok` of a git checkout.
const PACKAGE_SOURCE_LOCK: &str = ".cargo-ok";
const MAX_UNPACK_SIZE: u64 = 512 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: u64 = 20;
/// The time cargo gives the files `cargo package` makes, which a `.crate` has no time for.
const DETERMINISTIC_TIMESTAMP: i64 = 1153704088;

/// Values kept by the sha256 of their bytes: blaze's store, to which a `.crate` file is given
/// by its checksum.
pub trait Blobs: Send + Sync {
    fn get(&self, sha256: &[u8; 32]) -> Option<Vec<u8>>;
    fn has(&self, sha256: &[u8; 32]) -> bool;
    fn put(&self, sha256: &[u8; 32], bytes: &[u8]);
}

impl Blobs for ra_jit_store::Store {
    fn get(&self, sha256: &[u8; 32]) -> Option<Vec<u8>> {
        ra_jit_store::Store::get(self, sha256)
    }

    fn has(&self, sha256: &[u8; 32]) -> bool {
        ra_jit_store::Store::has(self, sha256)
    }

    fn put(&self, sha256: &[u8; 32], bytes: &[u8]) {
        self.insert(sha256, bytes);
    }
}

/// A package of a registry to fetch: the checksum is the lock file's, or the index's.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Package {
    pub name: String,
    pub version: Version,
    pub checksum: String,
}

impl std::fmt::Display for Package {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} v{}", self.name, self.version)
    }
}

/// Where a package was found, which a fetch counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum From {
    /// Already unpacked.
    Unpacked,
    Store,
    CargoCache,
    Network,
    /// Read where it is vendored, from a directory source.
    Vendored,
    /// In a checkout of a git source.
    Git,
}

/// The 32 bytes of a checksum written in hex.
pub fn parse_checksum(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0; 32];
    for (at, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(hex.get(at * 2..at * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|it| format!("{it:02x}")).collect()
}

/// The URL of a `.crate` file from a registry's `dl` template (cargo-util's `crate_url`).
pub fn crate_url(template: &str, name: &str, version: &str, checksum: &str) -> String {
    const MARKERS: [&str; 5] =
        ["{crate}", "{version}", "{prefix}", "{lowerprefix}", "{sha256-checksum}"];
    if !MARKERS.iter().any(|it| template.contains(it)) {
        return format!("{template}/{name}/{version}/download");
    }
    let prefix = dep_path(name, true);
    template
        .replace("{crate}", name)
        .replace("{version}", version)
        .replace("{prefix}", &prefix)
        .replace("{lowerprefix}", &prefix.to_lowercase())
        .replace("{sha256-checksum}", checksum)
}

impl Registry {
    /// Where a package is unpacked: `$CARGO_HOME/registry/src/<registry>/<name>-<version>`.
    pub fn package_dir(&self, name: &str, version: &Version) -> PathBuf {
        self.src_dir().join(format!("{name}-{version}"))
    }

    /// Fetches the packages, `jobs` at a time, and unpacks each where cargo does. Holds cargo's
    /// package cache lock while it does, as cargo holds it while it downloads.
    pub fn fetch(&self, packages: &[Package]) -> anyhow::Result<Vec<(PathBuf, From)>> {
        self.fetch_with_warm_registry(packages, warm_registry_fetch_enabled())
    }

    fn fetch_with_warm_registry(
        &self,
        packages: &[Package],
        enabled: bool,
    ) -> anyhow::Result<Vec<(PathBuf, From)>> {
        let _lock = crate::PackageCacheLock::acquire(&self.context().cargo_home)?;
        if enabled && let Some(out) = self.warm_registry_batch(packages) {
            warm_registry_fetch_marker();
            return Ok(out);
        }
        let next = AtomicUsize::new(0);
        let results: Vec<std::sync::Mutex<Option<anyhow::Result<(PathBuf, From)>>>> =
            packages.iter().map(|_| std::sync::Mutex::new(None)).collect();
        let jobs = self.context().net.jobs.max(1).min(packages.len().max(1));
        std::thread::scope(|scope| {
            for _ in 0..jobs {
                scope.spawn(|| {
                    loop {
                        let at = next.fetch_add(1, Ordering::Relaxed);
                        let Some(package) = packages.get(at) else { break };
                        let result = self.fetch_one(package);
                        if let Ok(mut slot) = results[at].lock() {
                            *slot = Some(result);
                        }
                    }
                });
            }
        });
        let mut out = Vec::with_capacity(packages.len());
        for (package, result) in packages.iter().zip(results) {
            let result = result.into_inner().ok().flatten().context("a fetch did not finish")?;
            out.push(result.with_context(|| format!("failed to download `{package}`"))?);
        }
        Ok(out)
    }

    /// Read-only sufficient proof of the existing `is_unpacked` early return. The caller
    /// holds the same package-cache lock as the original scheduler. Never call the mutating
    /// `is_unpacked` speculatively: even a failed removal can have changed the directory.
    fn warm_registry_batch(&self, packages: &[Package]) -> Option<Vec<(PathBuf, From)>> {
        const MAX_PACKAGES: usize = 4096;
        const MAX_MARKER_BYTES: u64 = 4096;
        if packages.is_empty() || packages.len() > MAX_PACKAGES {
            return None;
        }
        #[derive(serde_derive::Deserialize)]
        struct LockMetadata {
            v: u32,
        }
        let mut out = Vec::with_capacity(packages.len());
        for package in packages {
            let dst = self.package_dir(&package.name, &package.version);
            let file = std::fs::File::open(dst.join(PACKAGE_SOURCE_LOCK)).ok()?;
            let mut text = String::new();
            file.take(MAX_MARKER_BYTES + 1).read_to_string(&mut text).ok()?;
            if text.len() as u64 > MAX_MARKER_BYTES
                || !serde_json::from_str::<LockMetadata>(&text).is_ok_and(|it| it.v == 1)
            {
                return None;
            }
            out.push((dst, From::Unpacked));
        }
        Some(out)
    }

    /// One package, unpacked, and where its bytes came from. The caller holds the lock.
    pub fn fetch_one(&self, package: &Package) -> anyhow::Result<(PathBuf, From)> {
        let dst = self.package_dir(&package.name, &package.version);
        if is_unpacked(&dst)? {
            return Ok((dst, From::Unpacked));
        }
        let (bytes, from) = self.crate_bytes(package)?;
        unpack_package(&bytes, &dst)
            .with_context(|| format!("failed to unpack package `{package}`"))?;
        Ok((dst, from))
    }

    /// The bytes of a package's `.crate` file, checked against its checksum, and kept in the
    /// store and in cargo's cache.
    pub fn crate_bytes(&self, package: &Package) -> anyhow::Result<(Vec<u8>, From)> {
        let key = parse_checksum(&package.checksum)
            .with_context(|| format!("`{}` is not a sha256 checksum", package.checksum))?;
        let store = self.context().store.as_deref();
        let matches = |bytes: &[u8]| <[u8; 32]>::from(Sha256::digest(bytes)) == key;
        let cache_file = self.crate_file(&package.name, &package.version);
        if let Some(bytes) = store.and_then(|it| it.get(&key)).filter(|it| matches(it)) {
            if !self.is_local() && self.context().keep_crate_files {
                keep_crate_file(&cache_file, &bytes);
            }
            return Ok((bytes, From::Store));
        }
        match std::fs::read(&cache_file) {
            Ok(bytes) if !bytes.is_empty() && matches(&bytes) => {
                if let Some(store) = store {
                    store.put(&key, &bytes);
                }
                return Ok((bytes, From::CargoCache));
            }
            Ok(bytes) if !bytes.is_empty() && self.is_local() => {
                bail!("failed to verify the checksum of `{package}`")
            }
            _ if self.is_local() => bail!("failed to read `{}`", cache_file.display()),
            // Not what the index says, or cut short: downloaded again in its place.
            Ok(_) => _ = std::fs::remove_file(&cache_file),
            Err(_) => {}
        }
        if self.context().net.offline {
            bail!(
                "attempting to make an HTTP request, but --offline was specified \
                 (`{package}` is not on disk)"
            );
        }
        let config = self.config()?.context("config.json not found")?;
        let url =
            crate_url(&config.dl, &package.name, &package.version.to_string(), &package.checksum);
        let token = match config.auth_required {
            true => Some(self.sparse().context("a registry with a config.json")?.token(false)?),
            false => None,
        };
        let headers: Vec<(&str, &str)> =
            token.as_deref().map(|it| ("authorization", it)).into_iter().collect();
        let http = &self.context().http;
        let bytes = self.context().net.retry.run(|| {
            let answer = http.get(&url, &headers)?;
            if answer.status != 200 {
                return Err(HttpNotSuccessful::new(&url, &answer).into());
            }
            Ok(answer.body)
        })?;
        if !matches(&bytes) {
            bail!("failed to verify the checksum of `{package}`");
        }
        if let Some(store) = store {
            store.put(&key, &bytes);
        }
        if self.context().keep_crate_files {
            keep_crate_file(&cache_file, &bytes);
        }
        Ok((bytes, From::Network))
    }
}

// Exact first-use opt-in. No environment mutation or caching of filesystem proofs.
fn warm_registry_fetch_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var_os("RA_JIT_WARM_REGISTRY_FETCH").as_deref() == Some(std::ffi::OsStr::new("1"))
    })
}

fn warm_registry_fetch_marker() {
    static MARKER: std::sync::Once = std::sync::Once::new();
    MARKER.call_once(|| {
        let _ = std::io::stderr().lock().write_all(b"ra-jit: warm-registry-fetch-v85 batch\n");
    });
}

/// Writes a `.crate` into cargo's cache, as cargo would have, so that cargo finds it offline.
/// A cache that cannot be written is only said.
fn keep_crate_file(path: &Path, bytes: &[u8]) {
    if std::fs::metadata(path).is_ok_and(|it| it.len() > 0) {
        return;
    }
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| crate::write_whole(path, bytes));
    if let Err(e) = written {
        eprintln!("warning: failed to keep {}: {e}", path.display());
    }
}

/// Whether a package directory was unpacked whole by cargo 1.71 or later, or blaze; one left by
/// an interrupted or older unpacking is removed, to be unpacked again.
fn is_unpacked(dst: &Path) -> anyhow::Result<bool> {
    let ok = dst.join(PACKAGE_SOURCE_LOCK);
    match std::fs::read_to_string(&ok) {
        Ok(text) => {
            #[derive(serde_derive::Deserialize)]
            struct LockMetadata {
                v: u32,
            }
            if serde_json::from_str::<LockMetadata>(&text).is_ok_and(|it| it.v == 1) {
                return Ok(true);
            }
            std::fs::remove_dir_all(dst)
                .with_context(|| format!("failed to remove `{}`", dst.display()))?;
            Ok(false)
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => bail!("unable to read .cargo-ok file at {ok:?}: {e}"),
    }
}

/// Unpacks a `.crate` into `dst`, `<registry src>/<name>-<version>`, and marks it unpacked
/// (cargo's `unpack_package`).
pub fn unpack_package(tarball: &[u8], dst: &Path) -> anyhow::Result<u64> {
    std::fs::create_dir_all(dst)
        .with_context(|| format!("failed to create `{}`", dst.display()))?;
    let written = unpack(tarball, dst, &|_| true)?;
    set_generated_mtimes(dst);
    let path = dst.join(PACKAGE_SOURCE_LOCK);
    let mut ok = OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .open(&path)
        .with_context(|| format!("failed to open `{}`", path.display()))?;
    write!(ok, "{{\"v\":1}}")?;
    Ok(written)
}

/// Unpacks a `.crate` into `dst`, which is named `<name>-<version>`, with only the files
/// `include` takes by their path in the package, and no mark that it is unpacked: what
/// `cargo vendor` writes (cargo's `unpack_package_in`).
pub fn unpack_package_in(
    tarball: &[u8],
    dst: &Path,
    include: &dyn Fn(&Path) -> bool,
) -> anyhow::Result<u64> {
    let written = unpack(tarball, dst, include)?;
    set_generated_mtimes(dst);
    Ok(written)
}

/// The files `cargo package` writes, given one time whatever the tarball says
/// (rust-lang/cargo#16237).
fn set_generated_mtimes(dst: &Path) {
    for file in ["Cargo.lock", "Cargo.toml", ".cargo_vcs_info.json"] {
        let time = filetime::FileTime::from_unix_time(DETERMINISTIC_TIMESTAMP, 0);
        _ = filetime::set_file_mtime(dst.join(file), time);
    }
}

/// Unpacks a tarball whose every entry is under `<name>-<version>/`, files and directories
/// only, into the directory above `dst`, with no more than cargo lets one unpack to, but for
/// the entries `include` leaves out.
fn unpack(tarball: &[u8], dst: &Path, include: &dyn Fn(&Path) -> bool) -> anyhow::Result<u64> {
    let limit = MAX_UNPACK_SIZE.max(tarball.len() as u64 * MAX_COMPRESSION_RATIO);
    let gz = flate2::read::GzDecoder::new(tarball);
    let mut tar = Archive::new(LimitErrorReader { inner: gz, left: limit });
    #[cfg(unix)]
    tar.set_mask(umask());
    let prefix = dst.file_name().context("a package directory has a name")?;
    let parent = dst.parent().context("a package directory has a parent")?;
    let mut written = 0;
    for entry in tar.entries()? {
        let mut entry = entry.context("failed to iterate over archive")?;
        let entry_path = entry.path().context("failed to read entry path")?.into_owned();
        match entry_path.strip_prefix(prefix) {
            Ok(inside) if !include(inside) => continue,
            Ok(_) => {}
            Err(_) => bail!(
                "invalid tarball downloaded, contains a file at {entry_path:?} which isn't under {prefix:?}"
            ),
        }
        match entry.header().entry_type() {
            EntryType::Regular | EntryType::Directory => {}
            kind => bail!(
                "invalid tarball downloaded, contains an entry at {entry_path:?} with invalid type {kind:?}"
            ),
        }
        if entry_path.file_name().is_some_and(|it| it == PACKAGE_SOURCE_LOCK) {
            continue;
        }
        written += entry.size();
        let mut result = entry.unpack_in(parent).map_err(anyhow::Error::from);
        if cfg!(windows) && is_windows_reserved_path(&entry_path) {
            result = result.with_context(|| {
                format!(
                    "`{}` appears to contain a reserved Windows path, it cannot be extracted on Windows",
                    entry_path.display()
                )
            });
        }
        result.with_context(|| format!("failed to unpack entry at `{}`", entry_path.display()))?;
    }
    Ok(written)
}

/// A reader that fails once more than `left` bytes are read from it, as cargo's
/// `LimitErrorReader`.
struct LimitErrorReader<R> {
    inner: R,
    left: u64,
}

impl<R: Read> Read for LimitErrorReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let most =
            buf.len().min(usize::try_from(self.left.saturating_add(1)).unwrap_or(usize::MAX));
        let read = self.inner.read(&mut buf[..most])?;
        if read as u64 > self.left {
            return Err(io::Error::other("maximum limit reached when reading"));
        }
        self.left -= read as u64;
        Ok(read)
    }
}

/// The process's umask, read once (cargo's `get_umask`).
#[cfg(unix)]
fn umask() -> u32 {
    static UMASK: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *UMASK.get_or_init(|| {
        // SAFETY: `umask` cannot fail; it is set straight back to what it was, once, before
        // any unpacking.
        unsafe {
            let mask = libc::umask(0o022);
            libc::umask(mask);
            u32::from(mask)
        }
    })
}

/// Whether a path names one of Windows' devices, as cargo's `is_windows_reserved_path`.
fn is_windows_reserved_path(path: &Path) -> bool {
    const RESERVED: [&str; 22] = [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ];
    path.iter().filter_map(|it| it.to_str()).any(|component| {
        let stem = component.split('.').next().unwrap_or(component);
        RESERVED.contains(&stem.to_ascii_lowercase().as_str())
    })
}

/// The sha256 of some bytes in hex, as an index's `cksum` writes it.
pub fn checksum(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

#[cfg(test)]
mod warm_registry_readonly_v85_tests {
    use super::*;
    use std::sync::Arc;

    // Frozen accepted scheduler and fetch_one bodies, not a call through the candidate.
    impl Registry {
        fn fetch_v85_original(&self, packages: &[Package]) -> anyhow::Result<Vec<(PathBuf, From)>> {
            let _lock = crate::PackageCacheLock::acquire(&self.context().cargo_home)?;
            let next = AtomicUsize::new(0);
            let results: Vec<std::sync::Mutex<Option<anyhow::Result<(PathBuf, From)>>>> =
                packages.iter().map(|_| std::sync::Mutex::new(None)).collect();
            let jobs = self.context().net.jobs.max(1).min(packages.len().max(1));
            std::thread::scope(|scope| {
                for _ in 0..jobs {
                    scope.spawn(|| {
                        loop {
                            let at = next.fetch_add(1, Ordering::Relaxed);
                            let Some(package) = packages.get(at) else { break };
                            let result = self.fetch_one_v85_original(package);
                            if let Ok(mut slot) = results[at].lock() {
                                *slot = Some(result);
                            }
                        }
                    });
                }
            });
            let mut out = Vec::with_capacity(packages.len());
            for (package, result) in packages.iter().zip(results) {
                let result =
                    result.into_inner().ok().flatten().context("a fetch did not finish")?;
                out.push(result.with_context(|| format!("failed to download `{package}`"))?);
            }
            Ok(out)
        }

        fn fetch_one_v85_original(&self, package: &Package) -> anyhow::Result<(PathBuf, From)> {
            let dst = self.package_dir(&package.name, &package.version);
            if is_unpacked(&dst)? {
                return Ok((dst, From::Unpacked));
            }
            let (bytes, from) = self.crate_bytes(package)?;
            unpack_package(&bytes, &dst)
                .with_context(|| format!("failed to unpack package `{package}`"))?;
            Ok((dst, from))
        }
    }

    struct NoNetwork;
    impl crate::http::Http for NoNetwork {
        fn get(&self, _: &str, _: &[(&str, &str)]) -> anyhow::Result<crate::http::Response> {
            panic!("local registry fixture must never request the network")
        }
    }

    struct Fixture {
        home: PathBuf,
        registry: Registry,
    }

    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let home = std::env::temp_dir().join(format!(
                "blaze-warm-registry-v85-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            // create_dir rather than remove/reuse a potentially preexisting directory.
            std::fs::create_dir(&home).unwrap();
            let cache = home.join("local-registry");
            std::fs::create_dir(&cache).unwrap();
            let mut context = crate::Context::new(home.clone(), Arc::new(NoNetwork));
            context.net.offline = true;
            context.net.jobs = 3;
            let registry =
                Registry::local(blaze_source::SourceId::crates_io(), &cache, Arc::new(context));
            Self { home, registry }
        }

        fn marker(&self, package: &Package, bytes: &[u8]) {
            let dir = self.registry.package_dir(&package.name, &package.version);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(PACKAGE_SOURCE_LOCK), bytes).unwrap();
            std::fs::write(dir.join("native.bin"), [0, 255, 128, 10]).unwrap();
        }

        fn archive(&self, package: &mut Package) {
            let mut builder = tar::Builder::new(Vec::new());
            let bytes = [0, 255, 128, 10];
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_entry_type(tar::EntryType::Regular);
            builder
                .append_data(
                    &mut header,
                    format!("{}-{}/native.bin", package.name, package.version),
                    &bytes[..],
                )
                .unwrap();
            let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
            gz.write_all(&builder.into_inner().unwrap()).unwrap();
            let bytes = gz.finish().unwrap();
            package.checksum = checksum(&bytes);
            std::fs::write(self.registry.crate_file(&package.name, &package.version), bytes)
                .unwrap();
        }

        fn normalized(
            &self,
            result: anyhow::Result<Vec<(PathBuf, From)>>,
        ) -> Result<Vec<(PathBuf, From)>, String> {
            result
                .map(|out| {
                    out.into_iter()
                        .map(|(path, from)| {
                            (path.strip_prefix(&self.home).unwrap().to_owned(), from)
                        })
                        .collect()
                })
                .map_err(|error| format!("{error:#}").replace(self.home.to_str().unwrap(), "$HOME"))
        }

        fn snapshot(&self) -> Vec<(PathBuf, Option<Vec<u8>>)> {
            fn walk(root: &Path, dir: &Path, out: &mut Vec<(PathBuf, Option<Vec<u8>>)>) {
                for entry in std::fs::read_dir(dir).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        out.push((path.strip_prefix(root).unwrap().to_owned(), None));
                        walk(root, &path, out);
                    } else {
                        out.push((
                            path.strip_prefix(root).unwrap().to_owned(),
                            Some(std::fs::read(&path).unwrap()),
                        ));
                    }
                }
            }
            let mut out = Vec::new();
            walk(&self.home, &self.home, &mut out);
            out.sort();
            out
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.home);
        }
    }

    fn package(name: &str) -> Package {
        Package { name: name.into(), version: Version::new(1, 0, 0), checksum: "invalid".into() }
    }

    #[test]
    fn original_differential_empty_single_multiple_mixed_corrupt_missing_and_error() {
        for count in [0, 1, 3] {
            for state in [
                "warm",
                "missing",
                "corrupt",
                "mixed",
                "read-error",
                "checksum-error",
                "corrupt-archive",
                "missing-archive",
                "ordered-error-cleanup",
            ] {
                for enabled in [false, true] {
                    let original = Fixture::new();
                    let candidate = Fixture::new();
                    let mut packages: Vec<_> =
                        (0..count).map(|at| package(&format!("pkg{at}"))).collect();
                    for (at, package) in packages.iter_mut().enumerate() {
                        for fixture in [&original, &candidate] {
                            if state != "checksum-error" && state != "ordered-error-cleanup" {
                                fixture.archive(package);
                            }
                            if state == "corrupt-archive" {
                                std::fs::write(
                                    fixture.registry.crate_file(&package.name, &package.version),
                                    b"corrupt native archive",
                                )
                                .unwrap();
                            }
                            if state == "missing-archive" {
                                std::fs::remove_file(
                                    fixture.registry.crate_file(&package.name, &package.version),
                                )
                                .unwrap();
                            }
                            match state {
                                "warm" => fixture.marker(package, br#"{"v":1}"#),
                                "mixed" if at == 0 => fixture.marker(package, br#"{"v":1}"#),
                                "corrupt" => fixture.marker(package, br#"{"v":2}"#),
                                "ordered-error-cleanup" if at > 0 => {
                                    fixture.marker(
                                        package,
                                        b"invalid marker removed only by original",
                                    );
                                }
                                "read-error" => {
                                    let dir = fixture
                                        .registry
                                        .package_dir(&package.name, &package.version);
                                    std::fs::create_dir_all(dir.join(PACKAGE_SOURCE_LOCK)).unwrap();
                                }
                                _ => {}
                            }
                        }
                    }
                    let expected =
                        original.normalized(original.registry.fetch_v85_original(&packages));
                    let actual = candidate.normalized(
                        candidate.registry.fetch_with_warm_registry(&packages, enabled),
                    );
                    assert_eq!(actual, expected, "{state}/{count}/{enabled}");
                    assert_eq!(
                        candidate.snapshot(),
                        original.snapshot(),
                        "{state}/{count}/{enabled}"
                    );
                }
            }
        }
    }

    #[test]
    fn readonly_probe_retains_invalid_markers_and_native_bytes_and_bounds() {
        let fixture = Fixture::new();
        let package = package("native");
        let _lock = crate::PackageCacheLock::acquire(&fixture.home).unwrap();
        for marker in [
            b"broken".as_slice(),
            br#"{"v":0}"#,
            br#"{"v":1,"v":1}"#,
            br#"{"v":"1"}"#,
            &[255, 0, 128],
        ] {
            fixture.marker(&package, marker);
            let before = fixture.snapshot();
            assert!(fixture.registry.warm_registry_batch(std::slice::from_ref(&package)).is_none());
            assert_eq!(fixture.snapshot(), before);
        }
        fixture.marker(&package, br#"{"v":1,"unknown":true}"#);
        let expected =
            vec![(fixture.registry.package_dir(&package.name, &package.version), From::Unpacked)];
        assert_eq!(
            fixture.registry.warm_registry_batch(std::slice::from_ref(&package)),
            Some(expected)
        );
        assert!(fixture.registry.warm_registry_batch(&[]).is_none());
        assert!(fixture.registry.warm_registry_batch(&vec![package.clone(); 4097]).is_none());
        let huge = format!("{}{{\"v\":1}}", " ".repeat(4097));
        fixture.marker(&package, huge.as_bytes());
        assert!(fixture.registry.warm_registry_batch(std::slice::from_ref(&package)).is_none());
        assert!(
            is_unpacked(&fixture.registry.package_dir(&package.name, &package.version)).unwrap()
        );
    }

    #[test]
    fn removal_recovery_does_not_cache_warm_proofs() {
        let fixture = Fixture::new();
        let mut package = package("recover");
        fixture.archive(&mut package);
        fixture.marker(&package, br#"{"v":1}"#);
        assert_eq!(
            fixture
                .registry
                .fetch_with_warm_registry(std::slice::from_ref(&package), true)
                .unwrap()[0]
                .1,
            From::Unpacked
        );
        let dir = fixture.registry.package_dir(&package.name, &package.version);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(
            fixture
                .registry
                .fetch_with_warm_registry(std::slice::from_ref(&package), true)
                .unwrap()[0]
                .1,
            From::CargoCache
        );
        assert_eq!(std::fs::read(dir.join("native.bin")).unwrap(), [0, 255, 128, 10]);
        assert_eq!(
            fixture.registry.fetch_with_warm_registry(&[package], true).unwrap()[0].1,
            From::Unpacked
        );
    }

    #[test]
    fn successful_batches_preserve_input_order_duplicates_and_original_checksum_skip() {
        let fixture = Fixture::new();
        let a = package("alpha");
        let b = package("beta");
        fixture.marker(&a, br#"{"v":1}"#);
        fixture.marker(&b, br#"{"v":1}"#);
        let packages = [b.clone(), a, b];
        assert_eq!(
            fixture.registry.fetch_with_warm_registry(&packages, true).unwrap(),
            fixture.registry.fetch_v85_original(&packages).unwrap(),
        );
    }

    #[test]
    fn candidate_waits_for_same_package_cache_lock() {
        let fixture = Fixture::new();
        let package = package("locked");
        fixture.marker(&package, br#"{"v":1}"#);
        let lock = crate::PackageCacheLock::acquire(&fixture.home).unwrap();
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                started_tx.send(()).unwrap();
                done_tx.send(fixture.registry.fetch_with_warm_registry(&[package], true)).unwrap();
            });
            started_rx.recv().unwrap();
            assert!(done_rx.recv_timeout(std::time::Duration::from_millis(50)).is_err());
            drop(lock);
            assert!(done_rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap().is_ok());
        });
    }
}
