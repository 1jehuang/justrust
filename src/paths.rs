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
pub const RUSTC_SHIM_NAME: &str = "rustc";

pub fn home() -> Result<PathBuf> {
    if let Some(h) = std::env::var_os("JUSTRUST_HOME") {
        return Ok(PathBuf::from(h));
    }
    Ok(dirs::home_dir()
        .context("no home directory")?
        .join(".justrust"))
}

pub fn runs_dir() -> Result<PathBuf> {
    Ok(home()?.join("runs"))
}

pub fn index_file() -> Result<PathBuf> {
    Ok(home()?.join("index.jsonl"))
}

/// Make sure `~/.justrust/bin/shim/rustc` points at the running binary.
pub fn ensure_rustc_shim() -> Result<PathBuf> {
    let exe = std::env::current_exe()?.canonicalize()?;
    let dir = home()?.join("bin/shim");
    std::fs::create_dir_all(&dir)?;
    let link = dir.join(RUSTC_SHIM_NAME);
    replace_symlink(&exe, &link)?;
    Ok(link)
}

/// Atomically point `link` at `target`.
pub fn replace_symlink(target: &Path, link: &Path) -> Result<()> {
    if std::fs::read_link(link).ok().as_deref() == Some(target) {
        return Ok(());
    }
    let dir = link.parent().context("link has no parent")?;
    let name = link
        .file_name()
        .context("link has no name")?
        .to_string_lossy();
    let tmp = dir.join(format!(".{name}.{}", std::process::id()));
    let _ = std::fs::remove_file(&tmp);
    std::os::unix::fs::symlink(target, &tmp)?;
    std::fs::rename(&tmp, link)?;
    Ok(())
}

/// Find the real `cargo` on PATH, skipping any entry that resolves to justrust.
pub fn real_cargo() -> Result<PathBuf> {
    if let Some(p) = std::env::var_os("JUSTRUST_REAL_CARGO") {
        return Ok(PathBuf::from(p));
    }
    let me = std::env::current_exe()?.canonicalize()?;
    find_on_path("cargo", &me).context("could not find the real cargo on PATH")
}

pub fn find_on_path(name: &str, skip: &Path) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        let Ok(meta) = std::fs::metadata(&candidate) else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        use std::os::unix::fs::PermissionsExt;
        if meta.permissions().mode() & 0o111 == 0 {
            continue;
        }
        if candidate.canonicalize().ok().as_deref() == Some(skip) {
            continue;
        }
        return Some(candidate);
    }
    None
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
