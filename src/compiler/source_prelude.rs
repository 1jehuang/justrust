//! Embedded native prelude library sources (`libraries/prelude`, our own Rust
//! code). Each file is served to source preparation through a read-only
//! virtual directory, so every prelude file is an ordinary per-file
//! [`super::FileUnit`] graph input. Lanes add a file here plus `pub mod` in
//! `libraries/prelude/lib.rs`.
use std::path::{Path, PathBuf};

/// Virtual directory of the embedded library in source paths and diagnostics.
pub const PRELUDE_DIR: &str = "<prelude>";
/// Hidden crate-root module holding the library. `std::` and `core::` paths
/// map to it when no user item of that name is in scope.
pub const PRELUDE_MODULE: &str = "__justrust_std";
/// Attachment file loaded after the user crate root.
pub(super) const PRELUDE_ROOT: &str = "<prelude>/root.rs";

/// Virtual path (relative to [`PRELUDE_DIR`]) to embedded file text.
const PRELUDE_FILES: &[(&str, &str)] = &[
    ("root.rs", include_str!("../../libraries/prelude/root.rs")),
    (
        "__justrust_std/mod.rs",
        include_str!("../../libraries/prelude/lib.rs"),
    ),
    (
        "__justrust_std/option.rs",
        include_str!("../../libraries/prelude/option.rs"),
    ),
    (
        "__justrust_std/result.rs",
        include_str!("../../libraries/prelude/result.rs"),
    ),
    (
        "__justrust_std/prelude.rs",
        include_str!("../../libraries/prelude/prelude.rs"),
    ),
];

fn lookup(path: &Path) -> Option<&'static str> {
    let relative = path.strip_prefix(PRELUDE_DIR).ok()?;
    PRELUDE_FILES
        .iter()
        .find(|(name, _)| Path::new(name) == relative)
        .map(|(_, text)| *text)
}

/// Read-only reader over the embedded table. It has no filesystem capability.
pub(super) struct PreludeReader;
impl super::SourceReader for PreludeReader {
    fn canonicalize(&mut self, p: &Path) -> std::io::Result<PathBuf> {
        lookup(p)
            .map(|_| p.to_path_buf())
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))
    }
    fn is_file(&mut self, p: &Path) -> std::io::Result<bool> {
        Ok(lookup(p).is_some())
    }
    fn exists(&mut self, p: &Path) -> std::io::Result<bool> {
        Ok(lookup(p).is_some())
    }
    fn read(&mut self, p: &Path, limit: usize) -> std::io::Result<Vec<u8>> {
        let text = lookup(p).ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotFound))?;
        let bytes = text.as_bytes();
        Ok(bytes[..bytes.len().min(limit)].to_vec())
    }
}
