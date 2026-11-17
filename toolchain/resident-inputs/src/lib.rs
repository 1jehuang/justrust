//! Exact, conservative external Rust dependency snapshots for resident compilation.
//! Unsupported inputs are errors so callers can fall back to ordinary compilation.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

pub const SCHEMA: u32 = 1;
pub const MAX_FILES: usize = 2048;
pub const MAX_TOTAL_BYTES: u64 = 1 << 30;
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    schema: u32,
    externs: Vec<PathBuf>,
    directories: Vec<PathBuf>,
    files: Vec<Input>,
}

#[derive(Clone, Debug, Serialize, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
struct Input {
    path: PathBuf,
    bytes: u64,
    hash: String,
}

impl Manifest {
    /// Paths are eviction hints only, never proof that inputs remain unchanged.
    pub fn files(&self) -> impl Iterator<Item = &Path> {
        self.files.iter().map(|input| input.path.as_path())
    }

    pub fn directories(&self) -> impl Iterator<Item = &Path> {
        self.directories.iter().map(PathBuf::as_path)
    }

    /// Returns a content identity only after validating this manifest against disk.
    pub fn key(&self) -> Result<String, String> {
        self.verify()?;
        Ok(blake3::hash(&self.encoded()?).to_hex().to_string())
    }

    /// Recaptures content and directory membership, not merely file metadata.
    pub fn verify(&self) -> Result<(), String> {
        if self.schema != SCHEMA || self.externs.is_empty() {
            return Err("unsupported or empty manifest".into());
        }
        self.encoded()?;
        if snapshot(self.externs.clone(), self.directories.clone())? != *self {
            return Err("dependency inputs changed".into());
        }
        Ok(())
    }

    fn encoded(&self) -> Result<Vec<u8>, String> {
        // A bounded writer prevents oversized deserialized manifests from causing
        // an additional unbounded allocation while validating their encoding.
        struct Bounded(Vec<u8>);
        impl std::io::Write for Bounded {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                if bytes.len() >= MAX_MANIFEST_BYTES.saturating_sub(self.0.len()) {
                    return Err(std::io::Error::other("manifest budget exceeded"));
                }
                self.0.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let mut out = Bounded(Vec::new());
        serde_json::to_writer(&mut out, self).map_err(|e| e.to_string())?;
        Ok(out.0)
    }
}

/// Capture explicit extern files and every Rust artifact in dependency search paths.
/// Bare/name-only extern resolution is deliberately unsupported.
pub fn capture(args: &[String], cwd: &Path) -> Result<Option<Manifest>, String> {
    let mut externs = Vec::new();
    let mut searches = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if arg == "--extern" || arg == "-L" {
            i += 1;
            let value = args
                .get(i)
                .ok_or_else(|| format!("missing value for {arg}"))?;
            if arg == "--extern" {
                externs.push(value.as_str());
            } else {
                searches.push(value.as_str());
            }
        } else if let Some(value) = arg.strip_prefix("--extern=") {
            externs.push(value);
        } else if let Some(value) = arg.strip_prefix("-L") {
            searches.push(value);
        }
        i += 1;
    }
    if externs.is_empty() {
        return Ok(None);
    }
    let cwd = if cwd.is_absolute() {
        cwd.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|e| e.to_string())?
            .join(cwd)
    };
    let mut paths = Vec::new();
    let mut names = std::collections::BTreeMap::new();
    for value in externs {
        let (name, path) = value
            .split_once('=')
            .ok_or("extern requires explicit name=path")?;
        if name.is_empty()
            || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            || path.is_empty()
        {
            return Err("malformed extern input".into());
        }
        let path = normalize(&cwd.join(path))?;
        if let Some(old) = names.insert(name, path.clone()) {
            // Native Cargo can pass a matching rlib/rmeta pair when metadata
            // is not embedded. Both files are owned inputs, not ambiguity.
            let paired = old.parent() == path.parent()
                && old.file_stem() == path.file_stem()
                && matches!(
                    (
                        old.extension().and_then(|s| s.to_str()),
                        path.extension().and_then(|s| s.to_str())
                    ),
                    (Some("rlib"), Some("rmeta")) | (Some("rmeta"), Some("rlib"))
                );
            if old != path && !paired {
                return Err("ambiguous extern name".into());
            }
        }
        paths.push(path);
    }
    let mut dirs = Vec::new();
    for value in searches {
        let path = value
            .strip_prefix("dependency=")
            .ok_or("unsupported library search kind")?;
        if path.is_empty() {
            return Err("empty dependency search path".into());
        }
        dirs.push(normalize(&cwd.join(path))?);
    }
    snapshot(paths, dirs).map(Some)
}

fn normalize(path: &Path) -> Result<PathBuf, String> {
    if !path.is_absolute() {
        return Err("nonabsolute manifest path".into());
    }
    let mut out = PathBuf::new();
    // Inspect before removing '..', so symlink/.. cannot disguise traversal.
    for component in path.components() {
        match component {
            Component::CurDir => (),
            Component::ParentDir => {
                out.pop();
            }
            other => {
                out.push(other.as_os_str());
                let metadata =
                    fs::symlink_metadata(&out).map_err(|e| format!("{}: {e}", out.display()))?;
                if metadata.file_type().is_symlink() {
                    return Err("symlink dependency path".into());
                }
            }
        }
    }
    if out.to_str().is_none() {
        return Err("non UTF-8 dependency path".into());
    }
    Ok(out)
}

fn rust_artifact(path: &Path) -> bool {
    matches!(
        path.extension().and_then(|x| x.to_str()),
        Some("rlib" | "rmeta")
    )
}

fn membership(dirs: &[PathBuf]) -> Result<BTreeSet<PathBuf>, String> {
    let mut paths = BTreeSet::new();
    for dir in dirs {
        if normalize(dir)? != *dir || !fs::metadata(dir).map_err(|e| e.to_string())?.is_dir() {
            return Err("invalid dependency directory".into());
        }
        for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let kind = entry.file_type().map_err(|e| e.to_string())?;
            if kind.is_symlink() {
                return Err("symlink in dependency directory".into());
            }
            let path = entry.path();
            if path.extension().is_some_and(|x| x == "so") {
                return Err("dynamic/proc-macro dependency closure unsupported".into());
            }
            if rust_artifact(&path) {
                if !kind.is_file() {
                    return Err("nonregular Rust artifact".into());
                }
                paths.insert(path);
                if paths.len() > MAX_FILES {
                    return Err("file budget exceeded".into());
                }
            }
        }
    }
    Ok(paths)
}

fn unchanged(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        a.dev() == b.dev()
            && a.ino() == b.ino()
            && a.len() == b.len()
            && a.mtime() == b.mtime()
            && a.mtime_nsec() == b.mtime_nsec()
            && a.ctime() == b.ctime()
            && a.ctime_nsec() == b.ctime_nsec()
    }
    #[cfg(not(unix))]
    {
        a.len() == b.len()
            && a.modified().ok() == b.modified().ok()
            && a.created().ok() == b.created().ok()
    }
}

fn snapshot(mut externs: Vec<PathBuf>, mut directories: Vec<PathBuf>) -> Result<Manifest, String> {
    externs.sort();
    externs.dedup();
    directories.sort();
    directories.dedup();
    if externs.len() > MAX_FILES || directories.len() > MAX_FILES {
        return Err("input budget exceeded".into());
    }
    let members = membership(&directories)?;
    let mut paths = members.clone();
    for path in &externs {
        if !rust_artifact(path) || normalize(path)? != *path {
            return Err("invalid direct external artifact".into());
        }
        paths.insert(path.clone());
    }
    if paths.len() > MAX_FILES {
        return Err("file budget exceeded".into());
    }
    let mut total = 0u64;
    let mut files = Vec::new();
    let mut observed = Vec::new();
    for path in paths {
        normalize(&path)?;
        let before = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
        if !before.is_file() || before.file_type().is_symlink() {
            return Err("nonregular input".into());
        }
        if before.len() > MAX_TOTAL_BYTES - total {
            return Err("byte budget exceeded".into());
        }
        let mut file = File::open(&path).map_err(|e| e.to_string())?;
        if !unchanged(&before, &file.metadata().map_err(|e| e.to_string())?) {
            return Err("input raced open".into());
        }
        let mut hasher = blake3::Hasher::new();
        let mut bytes = 0u64;
        let mut buffer = [0u8; 65536];
        loop {
            let n = file.read(&mut buffer).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            bytes += n as u64;
            if bytes > MAX_TOTAL_BYTES - total {
                return Err("byte budget exceeded".into());
            }
            hasher.update(&buffer[..n]);
        }
        let after = file.metadata().map_err(|e| e.to_string())?;
        if bytes != before.len() || !unchanged(&before, &after) {
            return Err("input changed during read".into());
        }
        total += bytes;
        files.push(Input {
            path: path.clone(),
            bytes,
            hash: hasher.finalize().to_hex().to_string(),
        });
        observed.push((path, after));
    }
    if membership(&directories)? != members {
        return Err("dependency membership changed during capture".into());
    }
    for (path, metadata) in observed {
        normalize(&path)?;
        if !unchanged(
            &metadata,
            &fs::symlink_metadata(path).map_err(|e| e.to_string())?,
        ) {
            return Err("input changed during capture".into());
        }
    }
    let manifest = Manifest {
        schema: SCHEMA,
        externs,
        directories,
        files,
    };
    manifest.encoded()?;
    Ok(manifest)
}
