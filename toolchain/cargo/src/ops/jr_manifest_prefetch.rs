//! justrust: parse the manifests of already-unpacked registry packages in
//! parallel before cargo reads them one at a time.
//!
//! After resolving, every build turns each `PackageId` it needs into a
//! `Package` by reading `~/.cargo/registry/src/<reg>/<name>-<version>/Cargo.toml`
//! (TOML parse, deserialize, normalize, targets). On Jcode Desktop that is
//! ~720 manifests and ~250 ms of a ~500 ms no-op `cargo test`, all on one
//! thread. Here, while `Downloads` holds the package cache lock (so nothing
//! can unpack or delete those directories), we parse them on all cores and
//! park the results. `ops::read_package` takes a parked result only for the
//! exact manifest path and source it was parsed for, so the sequential code
//! path does everything else (`.cargo-ok` checks, last-use tracking,
//! checksums) exactly as before.
//!
//! Speculative: manifests that fail to parse are dropped without printing,
//! and the sequential read reports the error as usual. Whatever is not
//! taken is discarded when the download step ends. Disable with
//! `JUSTRUST_CARGO_PREFETCH=0`.

use crate::util::GlobalContext;
use crate::util::data_structures::HashMap;
use crate::workspace::{Package, SourceId};
use std::cell::Cell;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Below this many manifests, threads cost more than they save.
const MIN_BATCH: usize = 32;

static PARKED: Mutex<Option<HashMap<PathBuf, (SourceId, Package)>>> = Mutex::new(None);

thread_local! {
    static SPECULATIVE: Cell<bool> = const { Cell::new(false) };
}

/// True on prefetch threads: manifest diagnostics must not be printed there
/// (the sequential read prints them if the manifest is really used).
pub fn speculative() -> bool {
    SPECULATIVE.with(|s| s.get())
}

#[expect(
    clippy::disallowed_methods,
    reason = "justrust kill switch, read before any config is involved"
)]
pub fn enabled() -> bool {
    !matches!(
        std::env::var("JUSTRUST_CARGO_PREFETCH").as_deref(),
        Ok("0") | Ok("off") | Ok("false") | Ok("no")
    )
}

/// Clears the parked manifests when dropped.
pub struct Guard(());

impl Drop for Guard {
    fn drop(&mut self) {
        *PARKED.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

/// Parses `manifests` (manifest path, source) in parallel and parks the
/// packages that parsed cleanly. The caller must hold the package cache lock
/// for as long as the returned guard lives.
pub fn prefetch(gctx: &GlobalContext, manifests: Vec<(PathBuf, SourceId)>) -> Option<Guard> {
    if manifests.len() < MIN_BATCH || !enabled() {
        return None;
    }
    let _span = tracing::trace_span!("jr_manifest_prefetch", n = manifests.len()).entered();
    let threads = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
        .min(16);
    if threads < 2 {
        return None;
    }
    let next = AtomicUsize::new(0);
    let results: Mutex<HashMap<PathBuf, (SourceId, Package)>> = Mutex::new(HashMap::default());
    std::thread::scope(|s| {
        for _ in 0..threads {
            s.spawn(|| {
                SPECULATIVE.with(|f| f.set(true));
                let mut local = Vec::new();
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    let Some((path, sid)) = manifests.get(i) else {
                        break;
                    };
                    if let Ok(pkg) =
                        super::cargo_read_manifest::read_package_uncached(path, *sid, gctx)
                    {
                        local.push((path.clone(), (*sid, pkg)));
                    }
                }
                SPECULATIVE.with(|f| f.set(false));
                results.lock().unwrap().extend(local);
            });
        }
    });
    let results = results.into_inner().unwrap_or_else(|e| e.into_inner());
    tracing::debug!("jr_manifest_prefetch: parked {}", results.len());
    *PARKED.lock().unwrap_or_else(|e| e.into_inner()) = Some(results);
    Some(Guard(()))
}

/// Takes the parked package for exactly this manifest path and source.
pub fn take(path: &Path, source_id: SourceId) -> Option<Package> {
    let mut parked = PARKED.lock().unwrap_or_else(|e| e.into_inner());
    let map = parked.as_mut()?;
    let (sid, _) = map.get(path)?;
    if !sid.full_eq(source_id) {
        return None;
    }
    map.remove(path).map(|(_, pkg)| pkg)
}
