//! Pressure-only GC of stale Cargo build-script outputs in enrolled recent roots.
//! Keep the newest hash (including ties) for every package. Root leases and the
//! build lock remain authoritative even when individual outputs appear idle.
use super::{Probe, Registry, RootState};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

pub const IDLE: Duration = Duration::from_secs(2 * 3600);

fn package(name: &str) -> Option<&str> {
    let (package, hash) = name.rsplit_once('-')?;
    if package.is_empty()
        || !package
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        || hash.len() != 16
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(package)
}

// Fail closed for unreadable entries, symlinks (even internal ones), foreign
// ownership, mounts, and unusual file types. Inspect all descendants: build
// scripts may write nested files long after their parent directory's mtime.
fn tree_time(path: &Path, dev: u64) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    let m = std::fs::symlink_metadata(path)?;
    if m.dev() != dev || m.uid() != unsafe { libc::geteuid() } || (!m.is_dir() && !m.is_file()) {
        return Err(std::io::Error::other("unsafe build output entry"));
    }
    let mut newest = m
        .modified()?
        .max(m.created().unwrap_or(UNIX_EPOCH))
        .duration_since(UNIX_EPOCH)
        .map_err(std::io::Error::other)?
        .as_secs();
    // ctime catches a rewritten file whose mtime was restored by a build script.
    newest = newest.max(u64::try_from(m.ctime()).unwrap_or(u64::MAX));
    if m.is_dir() {
        for e in std::fs::read_dir(path)? {
            newest = newest.max(tree_time(&e?.path(), dev)?);
        }
    }
    Ok(newest)
}

fn build_dir(root: &Path) -> std::io::Result<(PathBuf, u64)> {
    use std::os::unix::fs::MetadataExt;
    let target = super::target_of(root).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
    let dev = std::fs::symlink_metadata(&target)?.dev();
    let mut dir = target;
    for component in ["debug", "build"] {
        dir.push(component);
        let m = std::fs::symlink_metadata(&dir)?;
        if !m.is_dir()
            || m.file_type().is_symlink()
            || m.dev() != dev
            || m.uid() != unsafe { libc::geteuid() }
            || dir.canonicalize()? != dir
        {
            return Err(std::io::Error::other("unsafe build output path"));
        }
    }
    Ok((dir, dev))
}

fn candidates(root: &Path, probe: &dyn Probe) -> std::io::Result<Vec<(PathBuf, u64)>> {
    let (build, dev) = build_dir(root)?;
    let mut groups: BTreeMap<String, Vec<(PathBuf, u64)>> = BTreeMap::new();
    for e in std::fs::read_dir(build)? {
        let e = e?;
        let name = e.file_name();
        let Some(pkg) = name.to_str().and_then(package) else {
            continue;
        };
        if !e.file_type()?.is_dir() {
            continue;
        }
        // An unsafe sibling makes its package ineligible rather than making an
        // older output look like the newest valid output for that package.
        let time = tree_time(&e.path(), dev).unwrap_or(u64::MAX);
        groups.entry(pkg.into()).or_default().push((e.path(), time));
    }
    let mut out = Vec::new();
    for entries in groups.into_values() {
        let newest = entries.iter().map(|e| e.1).max().unwrap_or(u64::MAX);
        if newest == u64::MAX {
            continue;
        }
        for (path, time) in entries {
            if time < newest && time.saturating_add(IDLE.as_secs()) <= probe.now() {
                out.push((path, time));
            }
        }
    }
    out.sort_by_key(|e| e.1);
    Ok(out)
}

pub fn collect(
    registry: &Registry,
    roots: &BTreeMap<PathBuf, RootState>,
    need: u64,
    dry: bool,
    probe: &dyn Probe,
    log: &mut dyn FnMut(String),
) -> u64 {
    let mut freed = 0u64;
    for root in roots.keys() {
        if freed >= need {
            break;
        }
        let Ok(_lock) = super::evict_lock(root) else {
            continue;
        };
        let Ok(state) = registry.fresh_state(root) else {
            continue;
        };
        if state.leases != 0 || probe.in_use_by_other(root) {
            continue;
        }
        let Ok(planned) = candidates(root, probe) else {
            continue;
        };
        for (path, time) in planned {
            if freed >= need {
                break;
            }
            // Recheck authority, age, newest retention and live consumers while
            // holding the same exclusion lock, including for dry plans.
            if probe.in_use_by_other(root)
                || !build_dir(root).is_ok_and(|(dir, dev)| {
                    path.parent() == Some(dir.as_path())
                        && path.canonicalize().is_ok_and(|p| p == path)
                        && tree_time(&path, dev).is_ok_and(|t| t == time)
                })
            {
                continue;
            }
            let bytes = probe.size(&path);
            if !dry && let Err(e) = std::fs::remove_dir_all(&path) {
                log(format!("evict: skip build output {} ({e})", path.display()));
                continue;
            }
            log(format!(
                "evict{}: {} stale build output {} (allocated_tree_bytes={bytes}, last_change={time})",
                if dry { " (dry run)" } else { "" },
                if dry { "would delete" } else { "deleted" },
                path.display()
            ));
            freed = freed.saturating_add(bytes);
        }
    }
    freed
}

#[cfg(test)]
include!("remote_evict_outputs_tests.rs");
