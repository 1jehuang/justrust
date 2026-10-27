//! Bounded acquisition of an owned source/discovery namespace for the CLI.
//! Two complete observations must agree before any compilation starts. This
//! detects observed host mutation, but is not a filesystem snapshot primitive
//! or permission to share artifacts. Compilation reads only the returned bytes.
use super::manifest::ManifestError;
use crate::compiler::source::SourceSnapshot;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: usize = 16 * 1024 * 1024;

pub(crate) fn directory(
    root: &Path,
    excluded_output: &Path,
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    capture_between(root, excluded_output, || {})
}

#[cfg(target_os = "linux")]
fn capture_between(
    root: &Path,
    excluded_output: &Path,
    between: impl FnOnce(),
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    let first = observe(root, excluded_output)?;
    between();
    let second = observe(root, excluded_output)?;
    if first.stamps != second.stamps || first.snapshot.digest() != second.snapshot.digest() {
        return Err(ManifestError(
            "input changed during native namespace capture; retry the command".into(),
        ));
    }
    Ok(second.snapshot)
}

#[cfg(not(target_os = "linux"))]
fn capture_between(
    _: &Path,
    _: &Path,
    _: impl FnOnce(),
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    Err(ManifestError(
        "unsupported: native namespace capture requires Linux".into(),
    ))
}

#[cfg(target_os = "linux")]
struct Observation {
    snapshot: Arc<SourceSnapshot>,
    stamps: BTreeMap<PathBuf, [u64; 8]>,
}

#[cfg(target_os = "linux")]
fn stamp(metadata: &std::fs::Metadata) -> [u64; 8] {
    use std::os::unix::fs::MetadataExt;
    [
        metadata.dev(),
        metadata.ino(),
        metadata.size(),
        metadata.mtime() as u64,
        metadata.mtime_nsec() as u64,
        metadata.ctime() as u64,
        metadata.ctime_nsec() as u64,
        metadata.mode() as u64,
    ]
}

#[cfg(target_os = "linux")]
fn observe(root: &Path, excluded_output: &Path) -> Result<Observation, ManifestError> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let infrastructure =
        |error| ManifestError(format!("infrastructure: native input capture: {error}"));
    let canonical = root.canonicalize().map_err(infrastructure)?;
    if canonical != root || root == Path::new("/") {
        return Err(ManifestError(
            "unsupported: capture root must be a canonical private directory".into(),
        ));
    }
    let mut remaining = MAX_BYTES;
    let mut stamps = BTreeMap::new();
    let mut files = Vec::new();
    let mut pending = vec![root.to_owned()];
    while let Some(path) = pending.pop() {
        if stamps.len() >= MAX_ENTRIES {
            return Err(ManifestError(
                "native input capture entry limit exceeded".into(),
            ));
        }
        let metadata = std::fs::symlink_metadata(&path).map_err(infrastructure)?;
        if metadata.file_type().is_symlink() {
            return Err(ManifestError(format!(
                "unsupported: native input capture does not follow symlinks: {}",
                path.display()
            )));
        }
        stamps.insert(path.clone(), stamp(&metadata));
        if metadata.is_dir() {
            // Cache-tagged directories are explicitly outside the source
            // namespace. A source attempting to import one therefore fails
            // closed instead of reading an omitted host file.
            if path != root {
                let tag = path.join("CACHEDIR.TAG");
                match std::fs::symlink_metadata(&tag) {
                    Ok(metadata) if metadata.is_file() => {
                        let mut bytes = Vec::new();
                        let file = std::fs::OpenOptions::new()
                            .read(true)
                            .custom_flags(0x20000)
                            .open(&tag)
                            .map_err(infrastructure)?;
                        if stamp(&file.metadata().map_err(infrastructure)?) != stamp(&metadata) {
                            return Err(ManifestError(
                                "input cache marker changed during capture".into(),
                            ));
                        }
                        file.take(64)
                            .read_to_end(&mut bytes)
                            .map_err(infrastructure)?;
                        if bytes.starts_with(b"Signature: 8a477f597d28d172789f06886806bc55") {
                            stamps.insert(tag, stamp(&metadata));
                            continue;
                        }
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                    Err(error) => return Err(infrastructure(error)),
                }
            }
            let mut entries = Vec::new();
            for entry in std::fs::read_dir(&path).map_err(infrastructure)? {
                let entry = entry.map_err(infrastructure)?.path();
                if entry == excluded_output
                    || entry == root.join(".git")
                    || entry == root.join("target")
                {
                    continue;
                }
                if entries
                    .len()
                    .saturating_add(pending.len())
                    .saturating_add(stamps.len())
                    >= MAX_ENTRIES
                {
                    return Err(ManifestError(
                        "native input capture entry limit exceeded".into(),
                    ));
                }
                entries.push(entry);
            }
            entries.sort();
            pending.extend(entries);
        } else if metadata.is_file() {
            let cost = path.as_os_str().len();
            if cost > remaining || metadata.len() > (remaining - cost) as u64 {
                return Err(ManifestError(
                    "native input capture byte limit exceeded".into(),
                ));
            }
            // Linux O_NOFOLLOW prevents a final-component symlink replacement
            // between metadata and open. Parent identities are checked below.
            let mut file = std::fs::OpenOptions::new()
                .read(true)
                .custom_flags(0x20000)
                .open(&path)
                .map_err(infrastructure)?;
            if stamp(&file.metadata().map_err(infrastructure)?) != stamp(&metadata) {
                return Err(ManifestError(
                    "input changed while opening native source".into(),
                ));
            }
            let mut bytes = Vec::new();
            (&mut file)
                .take((remaining - cost + 1) as u64)
                .read_to_end(&mut bytes)
                .map_err(infrastructure)?;
            if bytes.len() > remaining - cost {
                return Err(ManifestError(
                    "native input capture byte limit exceeded".into(),
                ));
            }
            remaining -= cost + bytes.len();
            if stamp(&file.metadata().map_err(infrastructure)?) != stamp(&metadata) {
                return Err(ManifestError(
                    "input changed while reading native source".into(),
                ));
            }
            files.push((path, bytes));
        } else {
            return Err(ManifestError(format!(
                "unsupported: native input capture requires regular files/directories: {}",
                path.display()
            )));
        }
    }
    // Namespace identities must remain unchanged across the entire acquisition,
    // not merely while each individual file happens to be open.
    for (path, expected) in &stamps {
        if stamp(&std::fs::symlink_metadata(path).map_err(infrastructure)?) != *expected {
            return Err(ManifestError(
                "input changed during native namespace capture".into(),
            ));
        }
    }
    let snapshot = SourceSnapshot::new(root.to_owned(), files).map_err(ManifestError)?;
    Ok(Observation { snapshot, stamps })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn capture_owns_bytes_and_rejects_mutation_and_symlinks() {
        let root = std::env::temp_dir().join(format!("jr-capture-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        let root = root.canonicalize().unwrap();
        let file = root.join("src/main.rs");
        std::fs::write(&file, "fn main() {}").unwrap();
        let excluded = root.join("target");
        let first = directory(&root, &excluded).unwrap();
        assert!(
            capture_between(&root, &excluded, || std::fs::write(
                &file,
                "fn main(){ loop {} }"
            )
            .unwrap())
            .is_err()
        );
        assert_ne!(
            first.digest(),
            directory(&root, &excluded).unwrap().digest()
        );
        assert!(
            capture_between(&root, &excluded, || std::fs::remove_file(&file).unwrap()).is_err()
        );
        std::os::unix::fs::symlink("missing", &file).unwrap();
        assert!(
            directory(&root, &excluded)
                .unwrap_err()
                .0
                .contains("symlinks")
        );
        std::fs::remove_file(&file).unwrap();
        let cache = root.join("previous-build");
        std::fs::create_dir(&cache).unwrap();
        std::fs::write(
            cache.join("CACHEDIR.TAG"),
            b"Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .unwrap();
        std::fs::File::create(cache.join("large-output"))
            .unwrap()
            .set_len((MAX_BYTES + 1) as u64)
            .unwrap();
        assert!(directory(&root, &excluded).is_ok());
        std::fs::write(cache.join("CACHEDIR.TAG"), b"not a cache signature").unwrap();
        assert!(directory(&root, &excluded).is_err());
        std::fs::remove_dir_all(cache).unwrap();
        std::fs::create_dir_all(&excluded).unwrap();
        std::os::unix::fs::symlink("missing", excluded.join("ignored")).unwrap();
        assert!(directory(&root, &excluded).is_ok());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn captured_driver_checks_builds_runs_after_host_inputs_disappear() {
        use crate::project::driver::{Command, Driver, Selection};
        let root = std::env::temp_dir().join(format!("jr-captured-driver-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='captured'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(
            root.join("src/main.rs"),
            "mod child; fn main() { assert_eq!(child::value(), 42); }",
        )
        .unwrap();
        std::fs::write(root.join("src/child.rs"), "pub fn value() -> i32 {42}").unwrap();
        let mut driver =
            Driver::capture(Some(&root.join("Cargo.toml")), &root, Selection::default()).unwrap();
        assert!(driver.ws.inputs.source_snapshot().is_some());
        driver.target_dir = root.join("output");
        std::fs::remove_file(root.join("Cargo.toml")).unwrap();
        std::fs::remove_dir_all(root.join("src")).unwrap();
        for workers in [1, 8] {
            driver.sel.workers = workers;
            for command in [Command::Check, Command::Build, Command::Run] {
                assert_eq!(driver.run(command).unwrap(), 0);
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
