//! justrust: skip re-resolving the whole workspace when nothing it depends
//! on changed since the last full resolve.
//!
//! Every `cargo build/check/test` normally resolves the entire workspace
//! (all members, all features, dev-deps) against the registry index just to
//! confirm that `Cargo.lock` is still up to date, then resolves again for
//! the requested packages. On Jcode Desktop the first resolve is ~110 ms of
//! a ~390 ms no-op run.
//!
//! After a full resolve that leaves `Cargo.lock` unchanged, we write a stamp
//! next to the lock file's target dir: a hash of every input the workspace
//! resolve reads. The next run computes the same hash cheaply (file contents
//! of the lock file, every workspace member and path-dependency manifest,
//! the cargo config files that apply, the cargo version, and the relevant
//! flags). If it matches, the lock file itself is used as the workspace
//! resolve and the second, targeted resolve still runs as usual against it,
//! so the build graph is computed exactly as before.
//!
//! Fail safe: any error computing the stamp, or any mismatch, runs the full
//! resolve. Disable with `JUSTRUST_CARGO_RESOLVE_STAMP=0`.

use crate::ops;
use crate::resolver::Resolve;
use crate::util::StableHasher;
use crate::util::errors::CargoResult;
use crate::workspace::Workspace;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

const STAMP_VERSION: u32 = 2;

#[expect(
    clippy::disallowed_methods,
    reason = "justrust kill switch, read before any config is involved"
)]
pub fn enabled() -> bool {
    !matches!(
        std::env::var("JUSTRUST_CARGO_RESOLVE_STAMP").as_deref(),
        Ok("0") | Ok("off") | Ok("false") | Ok("no")
    )
}

fn stamp_path(ws: &Workspace<'_>) -> PathBuf {
    // Outside the target/build dir, which tools and tests list exactly:
    // CARGO_HOME/.justrust-resolve/<hash of lock path + build dir>.
    let key = crate::util::hex::short_hash(&(
        ws.lock_root().as_path_unlocked().to_path_buf(),
        ws.build_dir().as_path_unlocked().to_path_buf(),
    ));
    ws.gctx()
        .home()
        .as_path_unlocked()
        .join(".justrust-resolve")
        .join(key)
}

fn hash_file(h: &mut StableHasher, path: &Path) -> std::io::Result<()> {
    path.hash(h);
    let bytes = std::fs::read(path)?;
    bytes.hash(h);
    Ok(())
}

/// Every input the workspace resolve reads that can change between runs.
fn compute(ws: &Workspace<'_>, lock: &Resolve) -> CargoResult<u64> {
    let gctx = ws.gctx();
    let mut h = StableHasher::new();
    STAMP_VERSION.hash(&mut h);
    crate::version().to_string().hash(&mut h);
    // Flags that change how the resolve behaves or whether it may write.
    gctx.offline_flag().hash(&mut h);
    gctx.locked_flag().hash(&mut h);
    format!("{:?}", gctx.cli_unstable()).hash(&mut h);
    ws.require_optional_deps().hash(&mut h);
    ws.is_ephemeral().hash(&mut h);
    format!("{:?}", ws.resolve_feature_unification()).hash(&mut h);

    // The lock file.
    hash_file(
        &mut h,
        &ws.lock_root().as_path_unlocked().join("Cargo.lock"),
    )?;
    // The root manifest (workspace table, [patch], [replace]).
    hash_file(&mut h, ws.root_manifest())?;
    // Every member manifest.
    let mut manifests: Vec<PathBuf> = ws.members().map(|p| p.manifest_path().to_owned()).collect();
    // Every path dependency in the lock file (sibling repos), and the
    // manifests of any workspace they belong to (inherited fields).
    for id in lock.iter().chain(lock.unused_patches().iter().copied()) {
        if let Some(dir) = id.source_id().local_path() {
            manifests.push(dir.join("Cargo.toml"));
        }
    }
    // Every [patch] path entry, used or not: a version bump in a patch
    // manifest can make it apply, which changes the lock file.
    for patches in ws.root_patch()?.values() {
        for p in patches {
            if let Some(dir) = p.dep.source_id().local_path() {
                manifests.push(dir.join("Cargo.toml"));
            }
        }
    }
    manifests.sort();
    manifests.dedup();
    for m in &manifests {
        hash_file(&mut h, m)?;
        // Workspace roots that a path dependency inherits from.
        for anc in m.parent().into_iter().flat_map(|p| p.ancestors().skip(1)) {
            let cand = anc.join("Cargo.toml");
            if cand.is_file() && !manifests.contains(&cand) {
                hash_file(&mut h, &cand)?;
                break;
            }
        }
    }
    // Cargo config files (source replacement, registries, patch, paths)
    // from the cwd up to the root, plus CARGO_HOME.
    let mut configs = Vec::new();
    for dir in gctx.cwd().ancestors() {
        for name in ["config.toml", "config"] {
            let p = dir.join(".cargo").join(name);
            if p.is_file() {
                configs.push(p);
            }
        }
    }
    for name in ["config.toml", "config"] {
        let p = gctx.home().as_path_unlocked().join(name);
        if p.is_file() {
            configs.push(p);
        }
    }
    for c in &configs {
        hash_file(&mut h, c)?;
    }
    // Environment that maps onto resolver-relevant config.
    let mut env: Vec<(String, String)> = gctx
        .env()
        .filter(|(k, _)| {
            k.starts_with("CARGO_REGISTRIES_")
                || k.starts_with("CARGO_REGISTRY_")
                || k.starts_with("CARGO_SOURCE_")
                || k.starts_with("CARGO_PATCH_")
                || k.starts_with("CARGO_RESOLVER_")
                || k.starts_with("CARGO_UNSTABLE_")
                || *k == "CARGO_NET_OFFLINE"
                || *k == "__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS"
        })
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    env.sort();
    env.hash(&mut h);
    Ok(Hasher::finish(&h))
}

/// The lock file as the workspace resolve, if the stamp proves that a full
/// resolve would reproduce it unchanged.
pub fn try_cached(ws: &Workspace<'_>) -> Option<Resolve> {
    if !enabled() || ws.is_ephemeral() || !ws.require_optional_deps() {
        return None;
    }
    // The full resolve also reports unused `[replace]` entries and unused
    // patches. Keep those warnings exact: workspaces that have either
    // always take the full path.
    if !ws.root_replace().is_empty() {
        return None;
    }
    let stored = std::fs::read_to_string(stamp_path(ws)).ok()?;
    let lock = ops::load_pkg_lockfile(ws).ok()??;
    if !lock.unused_patches().is_empty() {
        return None;
    }
    let now = compute(ws, &lock).ok()?;
    if stored.trim() == format!("{now:016x}") {
        tracing::debug!("justrust resolve stamp matches, skipping workspace resolve");
        Some(lock)
    } else {
        None
    }
}

/// After a full resolve: record the stamp for the lock file now on disk.
pub fn record(ws: &Workspace<'_>) {
    if !enabled() || ws.is_ephemeral() || !ws.require_optional_deps() {
        return;
    }
    let path = stamp_path(ws);
    let result = (|| -> CargoResult<()> {
        let Some(lock) = ops::load_pkg_lockfile(ws)? else {
            return Ok(());
        };
        let v = compute(ws, &lock)?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, format!("{v:016x}\n"))?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&path);
    }
}
