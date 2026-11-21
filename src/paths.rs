//! Filesystem layout for recorded runs.
//!
//! ```text
//! ~/.justrust/                      (override with JUSTRUST_HOME)
//!   bin/justrust-rustc              symlink to the justrust binary, used as RUSTC
//!   index.jsonl                     one summary line per finished run
//!   runs/<run-id>/meta.json         command, cwd, agent session, git, timing
//!   runs/<run-id>/output.jsonl      every cargo output line with a timestamp
//!   runs/<run-id>/units.jsonl       one record per rustc invocation (from the shim)
//!   runs/<run-id>/samples.jsonl     periodic CPU, memory, and pressure samples
//!   runs/<run-id>/processes.json    per-process CPU and memory for the build tree
//!   runs/<run-id>/summary.json      computed breakdown of where the time went
//! ```

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

/// Cargo runs `<home>/bin/shim/rustc` as `RUSTC`. The binary dispatches on
/// its file name, so the shim must be named exactly `rustc`.
pub const RUSTC_SHIM_NAME: &str = if cfg!(windows) { "rustc.exe" } else { "rustc" };

pub fn home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("JUSTRUST_HOME") {
        return Ok(PathBuf::from(h));
    }
    Ok(dirs::home_dir()
        .context("no home directory")?
        .join(".justrust"))
}

/// `JUSTRUST_DEPCACHE_NAMESPACE`: run with private, initially empty
/// dependency and local unit caches under `cache/ns/<name>/`, and without
/// the peer tier (which would serve the default cache). The public prebuilt
/// store still applies. Benchmarks use it to measure what a new account's
/// first build gets on hosted machines that already have warm caches.
/// `Err` for a name that is not 1-64 of `[A-Za-z0-9_-]`, so a typo never
/// falls back to the warm default cache.
pub fn cache_namespace() -> Result<Option<String>, String> {
    let v = std::env::var_os("JUSTRUST_DEPCACHE_NAMESPACE");
    parse_cache_namespace(v.as_deref().map(|v| v.to_string_lossy()).as_deref())
}

fn parse_cache_namespace(v: Option<&str>) -> Result<Option<String>, String> {
    match v {
        None | Some("") => Ok(None),
        Some(v)
            if v.len() <= 64
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') =>
        {
            Ok(Some(v.to_owned()))
        }
        Some(v) => Err(format!("invalid JUSTRUST_DEPCACHE_NAMESPACE {v:?}")),
    }
}

/// Parent of the dependency and local unit caches (`cache/`, or
/// `cache/ns/<name>/` under [`cache_namespace`]). `None` for an invalid
/// namespace: caching is off rather than silently warm.
pub fn cache_root() -> Option<PathBuf> {
    cache_root_in(&home().ok()?, cache_namespace())
}

fn cache_root_in(home: &Path, ns: Result<Option<String>, String>) -> Option<PathBuf> {
    let base = home.join("cache");
    match ns {
        Ok(None) => Some(base),
        Ok(Some(ns)) => Some(base.join("ns").join(ns)),
        Err(_) => None,
    }
}

pub fn runs_dir() -> Result<PathBuf> {
    Ok(home()?.join("runs"))
}

pub fn index_file() -> Result<PathBuf> {
    Ok(home()?.join("index.jsonl"))
}

/// Make sure a `rustc` shim pointing at the running binary exists, and
/// return it. One shim dir per binary (`bin/shim/<hash of the binary
/// path>/rustc`): a single shared link was repointed by every build, so
/// two justrust binaries building at once (an installed one and a bench or
/// development build) ran each other's rustc shims, mixing depcache
/// schemes inside one build. `bin/shim/rustc` is still kept pointing at
/// the most recent binary for anything that runs it directly.
pub fn ensure_rustc_shim() -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let base = home()?.join("bin/shim");
    let mut h = crate::depcache::H128::new();
    h.field(exe.as_os_str().as_encoded_bytes());
    let dir = base.join(&h.hex()[..16]);
    std::fs::create_dir_all(&dir)?;
    let link = dir.join(RUSTC_SHIM_NAME);
    replace_symlink(&exe, &link)?;
    let _ = replace_symlink(&exe, &base.join(RUSTC_SHIM_NAME));
    Ok(link)
}

/// Windows: the shim and the `cargo` proxy are hard links (or copies) of
/// justrust.exe, not symlinks (those need developer mode). Their names do
/// not say justrust, so each gets a sidecar file (`cargo.exe` ->
/// `cargo.exe.justrust`) that marks it as ours.
pub fn copy_mark(link: &Path) -> PathBuf {
    let mut s = link.as_os_str().to_owned();
    s.push(".justrust");
    PathBuf::from(s)
}

/// Atomically point `link` at `target`. On Windows a hard link or copy
/// of `target` (replaced only when it differs), marked as justrust's.
pub fn replace_symlink(target: &Path, link: &Path) -> Result<()> {
    if std::fs::read_link(link).ok().as_deref() == Some(target) {
        return Ok(());
    }
    #[cfg(windows)]
    if same_file_contents(target, link) {
        return Ok(());
    }
    let dir = link.parent().context("link has no parent")?;
    let name = link
        .file_name()
        .context("link has no name")?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    crate::sys::symlink(target, &tmp)?;
    if let Err(e) = std::fs::rename(&tmp, link) {
        // Windows cannot replace a running executable: move it aside first.
        #[cfg(windows)]
        {
            let old = dir.join(format!(".{name}.old.{}", std::process::id()));
            if std::fs::rename(link, &old).is_ok() && std::fs::rename(&tmp, link).is_ok() {
                let _ = std::fs::remove_file(&old);
                let _ = std::fs::write(copy_mark(link), b"");
                return Ok(());
            }
        }
        let _ = std::fs::remove_file(&tmp);
        return Err(e.into());
    }
    #[cfg(windows)]
    let _ = std::fs::write(copy_mark(link), b"");
    Ok(())
}

/// Same size and modification time, or the same file (a hard link).
#[cfg(windows)]
fn same_file_contents(a: &Path, b: &Path) -> bool {
    match (std::fs::metadata(a), std::fs::metadata(b)) {
        (Ok(x), Ok(y)) => x.len() == y.len() && x.modified().ok() == y.modified().ok(),
        _ => false,
    }
}

/// Find the real `cargo` on PATH, skipping any entry that resolves to justrust.
pub fn real_cargo() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("JUSTRUST_REAL_CARGO") {
        return Ok(PathBuf::from(p));
    }
    // Atomic installation can unlink this running executable on Linux.
    // current_exe then names a deleted inode, but PATH's proxy detection is
    // still sufficient to locate Cargo without canonicalizing that inode.
    let exe = std::env::current_exe()?;
    let me = exe.canonicalize().unwrap_or(exe);
    find_on_path("cargo", &me).context("could not find the real cargo on PATH")
}

pub fn find_on_path(name: &str, skip: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let name = crate::sys::exe_name(name);
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(&name);
        if !crate::sys::is_executable(&candidate) {
            continue;
        }
        if candidate.canonicalize().ok().as_deref() == Some(skip) || is_justrust(&candidate) {
            continue;
        }
        return Some(candidate);
    }
    None
}

/// True if `path` resolves to a justrust binary (for example the `cargo` proxy
/// installed by `justrust install`), so it is never mistaken for the real tool.
pub fn is_justrust(path: &Path) -> bool {
    if cfg!(windows) && copy_mark(path).exists() {
        return true;
    }
    path.canonicalize()
        .ok()
        .and_then(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().starts_with("justrust"))
        })
        .unwrap_or(false)
}

pub fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// Append one line to a file shared by concurrent writers. A single `write`
/// on an `O_APPEND` file keeps lines from interleaving.
pub fn append_line(path: &Path, line: &str) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let mut buf = Vec::with_capacity(line.len() + 1);
    buf.extend_from_slice(line.as_bytes());
    buf.push(b'\n');
    f.write_all(&buf)?;
    Ok(())
}

#[cfg(test)]
mod cache_namespace_tests {
    use super::*;

    #[test]
    fn namespace_selects_a_separate_cache_root_and_rejects_paths() {
        let h = Path::new("/h");
        let root = |v| cache_root_in(h, parse_cache_namespace(v));
        assert_eq!(root(None), Some(h.join("cache")));
        assert_eq!(root(Some("")), Some(h.join("cache")));
        assert_eq!(root(Some("cold-1_a")), Some(h.join("cache/ns/cold-1_a")));
        // A typo or a path must never fall back to the warm default cache.
        for bad in ["..", "../x", "a/b", "x y", ".", &"n".repeat(65)] {
            assert_eq!(root(Some(bad)), None, "{bad}");
        }
    }
}
