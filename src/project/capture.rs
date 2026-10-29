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
    directories(root, &[], excluded_output)
}

pub(crate) fn directories(
    root: &Path,
    additional: &[PathBuf],
    excluded_output: &Path,
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    capture_roots_between(root, additional, excluded_output, || {})
}

#[cfg(test)]
fn directories_and_files(
    root: &Path,
    additional: &[PathBuf],
    files: &[PathBuf],
    excluded_output: &Path,
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    capture_inputs_between(root, additional, files, excluded_output, || {})
}

pub(crate) fn directories_files_excluding(
    root: &Path,
    additional: &[PathBuf],
    files: &[PathBuf],
    excluded_inputs: &[PathBuf],
    excluded_output: &Path,
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    capture_filtered_between(
        root,
        additional,
        files,
        excluded_inputs,
        excluded_output,
        || {},
    )
}

#[cfg(all(test, target_os = "linux"))]
fn capture_between(
    root: &Path,
    excluded_output: &Path,
    between: impl FnOnce(),
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    capture_roots_between(root, &[], excluded_output, between)
}

fn capture_roots_between(
    root: &Path,
    additional: &[PathBuf],
    excluded_output: &Path,
    between: impl FnOnce(),
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    capture_inputs_between(root, additional, &[], excluded_output, between)
}

fn capture_inputs_between(
    root: &Path,
    additional: &[PathBuf],
    files: &[PathBuf],
    excluded_output: &Path,
    between: impl FnOnce(),
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    capture_filtered_between(root, additional, files, &[], excluded_output, between)
}

#[cfg(target_os = "linux")]
fn capture_filtered_between(
    root: &Path,
    additional: &[PathBuf],
    files: &[PathBuf],
    excluded_inputs: &[PathBuf],
    excluded_output: &Path,
    between: impl FnOnce(),
) -> Result<Arc<SourceSnapshot>, ManifestError> {
    let first = observe(root, additional, files, excluded_inputs, excluded_output)?;
    between();
    let second = observe(root, additional, files, excluded_inputs, excluded_output)?;
    if first.stamps != second.stamps || first.snapshot.digest() != second.snapshot.digest() {
        return Err(ManifestError(
            "input changed during native namespace capture; retry the command".into(),
        ));
    }
    Ok(second.snapshot)
}

#[cfg(not(target_os = "linux"))]
fn capture_filtered_between(
    _: &Path,
    _: &[PathBuf],
    _: &[PathBuf],
    _: &[PathBuf],
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
fn observe(
    root: &Path,
    additional: &[PathBuf],
    selected_files: &[PathBuf],
    excluded_inputs: &[PathBuf],
    excluded_output: &Path,
) -> Result<Observation, ManifestError> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let infrastructure =
        |error| ManifestError(format!("infrastructure: native input capture: {error}"));
    let roots = SourceSnapshot::normalized_roots(root, additional).map_err(ManifestError)?;
    if selected_files.len() > MAX_ENTRIES {
        return Err(ManifestError(
            "native input capture entry limit exceeded".into(),
        ));
    }
    let mut namespace_roots: std::collections::BTreeSet<_> = additional.iter().cloned().collect();
    for file in selected_files {
        namespace_roots.insert(
            file.parent()
                .ok_or_else(|| ManifestError("capture file has no parent".into()))?
                .to_owned(),
        );
    }
    let namespace_roots: Vec<_> = namespace_roots.into_iter().collect();
    SourceSnapshot::normalized_roots(root, &namespace_roots).map_err(ManifestError)?;
    for root in std::iter::once(root).chain(additional.iter().map(PathBuf::as_path)) {
        let canonical = root.canonicalize().map_err(infrastructure)?;
        if canonical != root
            || !std::fs::symlink_metadata(root)
                .map_err(infrastructure)?
                .is_dir()
        {
            return Err(ManifestError(
                "unsupported: capture root must be a canonical private directory".into(),
            ));
        }
    }
    let mut remaining = MAX_BYTES;
    let mut stamps = BTreeMap::new();
    let mut files = Vec::new();
    let mut pending: Vec<_> = roots
        .iter()
        .map(|root| (root.clone(), root.clone()))
        .collect();
    for file in selected_files {
        if roots.iter().any(|root| file.starts_with(root)) {
            continue;
        }
        if file.canonicalize().map_err(infrastructure)? != *file
            || !std::fs::symlink_metadata(file)
                .map_err(infrastructure)?
                .is_file()
        {
            return Err(ManifestError(
                "unsupported: capture files must be canonical regular files".into(),
            ));
        }
        let parent = file.parent().unwrap().to_owned();
        stamps.insert(
            parent.clone(),
            stamp(&std::fs::symlink_metadata(&parent).map_err(infrastructure)?),
        );
        pending.push((parent, file.clone()));
    }
    while let Some((capture_root, path)) = pending.pop() {
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
            if path != capture_root {
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
                            if stamps.len() >= MAX_ENTRIES {
                                return Err(ManifestError(
                                    "native input capture entry limit exceeded".into(),
                                ));
                            }
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
                    || excluded_inputs.contains(&entry)
                    || entry == capture_root.join(".git")
                    || entry == capture_root.join("target")
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
            pending.extend(entries.into_iter().map(|path| (capture_root.clone(), path)));
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
    let snapshot = SourceSnapshot::with_roots(root.to_owned(), &namespace_roots, files)
        .map_err(ManifestError)?;
    Ok(Observation { snapshot, stamps })
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    #[test]
    fn selected_archives_are_owned_without_sweeping_the_cache() {
        use crate::compiler::source::SourceReader;
        let base = std::env::temp_dir().join(format!("jr-capture-archives-{}", std::process::id()));
        let root = base.join("app");
        let cache = base.join("cache");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&cache).unwrap();
        let root = root.canonicalize().unwrap();
        let file = cache.canonicalize().unwrap().join("dep.crate");
        std::fs::write(root.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(&file, b"owned archive").unwrap();
        std::os::unix::fs::symlink("missing", cache.join("unrelated")).unwrap();
        let files = [file.clone()];
        let output = root.join("target");
        let frozen = directories_and_files(&root, &[], &files, &output).unwrap();
        assert!(!frozen.reader().exists(&cache.join("unrelated")).unwrap());
        assert!(
            capture_inputs_between(&root, &[], &files, &output, || {
                std::fs::write(&file, b"changed archive").unwrap();
            })
            .is_err()
        );
        assert!(
            capture_inputs_between(&root, &[], &files, &output, || {
                std::fs::remove_file(&file).unwrap();
            })
            .is_err()
        );
        assert_eq!(
            frozen.reader().read(&file, MAX_BYTES).unwrap(),
            b"owned archive"
        );
        std::os::unix::fs::symlink(root.join("main.rs"), &file).unwrap();
        assert!(directories_and_files(&root, &[], &files, &output).is_err());
        std::fs::remove_file(&file).unwrap();
        std::fs::File::create(&file)
            .unwrap()
            .set_len((MAX_BYTES / 2) as u64)
            .unwrap();
        std::fs::File::create(root.join("large"))
            .unwrap()
            .set_len((MAX_BYTES / 2) as u64)
            .unwrap();
        assert!(
            directories_and_files(&root, &[], &files, &output)
                .unwrap_err()
                .0
                .contains("byte limit")
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn cache_markers_cannot_overrun_the_aggregate_entry_limit() {
        let root =
            std::env::temp_dir().join(format!("jr-capture-marker-limit-{}", std::process::id()));
        let cache = root.join("aaa-cache");
        std::fs::create_dir_all(&cache).unwrap();
        std::fs::write(
            cache.join("CACHEDIR.TAG"),
            b"Signature: 8a477f597d28d172789f06886806bc55\n",
        )
        .unwrap();
        // Sorted traversal visits this cache last. Without charging its marker,
        // the acquisition would silently record MAX_ENTRIES + 1 observations.
        for index in 0..MAX_ENTRIES - 2 {
            std::fs::write(root.join(format!("zzz-{index}")), "").unwrap();
        }
        let root = root.canonicalize().unwrap();
        assert!(
            directory(&root, &root.join("target"))
                .unwrap_err()
                .0
                .contains("entry limit")
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn multi_root_capture_is_closed_bounded_and_rejects_cross_root_mutation() {
        use crate::compiler::source::SourceReader;
        let base = std::env::temp_dir().join(format!("jr-multi-capture-{}", std::process::id()));
        let app = base.join("app");
        let dep = base.join("dep");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&dep).unwrap();
        let app = app.canonicalize().unwrap();
        let dep = dep.canonicalize().unwrap();
        let source = dep.join("lib.rs");
        std::fs::write(app.join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(&source, "pub fn value() -> i32 { 42 }").unwrap();
        let output = app.join("target");
        let roots = [dep.clone()];
        let frozen = directories(&app, &roots, &output).unwrap();
        assert!(
            capture_roots_between(&app, &roots, &output, || {
                std::fs::write(&source, "pub fn value() -> i32 { 43 }").unwrap();
            })
            .is_err()
        );
        assert!(
            capture_roots_between(&app, &roots, &output, || {
                std::fs::write(dep.join("new.rs"), "").unwrap();
            })
            .is_err()
        );
        assert!(
            capture_roots_between(&app, &roots, &output, || {
                std::fs::remove_file(&source).unwrap();
            })
            .is_err()
        );
        // Owned reads and negative answers survive live file deletion/creation.
        assert!(
            frozen
                .reader()
                .read(&source, 100)
                .unwrap()
                .ends_with(b"42 }")
        );
        assert!(!frozen.reader().exists(&dep.join("new.rs")).unwrap());
        assert!(frozen.reader().read(&base.join("escape.rs"), 100).is_err());
        assert!(
            frozen
                .reader()
                .canonicalize(&app.join("../escape.rs"))
                .is_err()
        );
        assert!(!frozen.contains(&app.join("../unowned")));
        std::os::unix::fs::symlink(app.join("main.rs"), &source).unwrap();
        assert!(
            directories(&app, &roots, &output)
                .unwrap_err()
                .0
                .contains("symlinks")
        );
        std::fs::remove_file(&source).unwrap();
        std::fs::write(&source, "").unwrap();
        let alias = base.join("alias");
        std::os::unix::fs::symlink(&dep, &alias).unwrap();
        assert!(directories(&app, &[alias], &output).is_err());
        assert!(directories(&app, &[PathBuf::from("/")], &output).is_err());
        assert!(
            directories(&app, &vec![dep.clone(); 64], &output)
                .unwrap_err()
                .0
                .contains("root limit")
        );
        for root in [&app, &dep] {
            std::fs::File::create(root.join("large"))
                .unwrap()
                .set_len((MAX_BYTES / 2) as u64)
                .unwrap();
        }
        assert!(
            directories(&app, &roots, &output)
                .unwrap_err()
                .0
                .contains("byte limit")
        );
        std::fs::remove_file(app.join("large")).unwrap();
        std::fs::remove_file(dep.join("large")).unwrap();
        for index in 0..MAX_ENTRIES / 2 {
            for root in [&app, &dep] {
                std::fs::write(root.join(format!("entry-{index}")), "").unwrap();
            }
        }
        assert!(
            directories(&app, &roots, &output)
                .unwrap_err()
                .0
                .contains("entry limit")
        );
        std::fs::remove_dir_all(base).unwrap();
    }

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
        if !crate::project::in_clean_test_process(
            "project::capture::tests::captured_driver_checks_builds_runs_after_host_inputs_disappear",
        ) {
            return;
        }
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

    #[test]
    fn captured_sibling_path_dependencies_run_after_both_roots_disappear() {
        if !crate::project::in_clean_test_process(
            "project::capture::tests::captured_sibling_path_dependencies_run_after_both_roots_disappear",
        ) {
            return;
        }
        use crate::project::driver::{Command, Driver, Selection};
        let base =
            std::env::temp_dir().join(format!("jr-captured-siblings-{}", std::process::id()));
        let app = base.join("app");
        let dep = base.join("dep");
        std::fs::create_dir_all(app.join("src")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname='app'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={path='../dep'}\n").unwrap();
        std::fs::write(
            dep.join("Cargo.toml"),
            "[package]\nname='dep'\nversion='0.1.0'\nedition='2024'\n",
        )
        .unwrap();
        std::fs::write(
            app.join("src/main.rs"),
            "fn main() { assert_eq!(dep::value(), 42); }",
        )
        .unwrap();
        std::fs::write(dep.join("src/lib.rs"), "pub fn value() -> i32 { 42 }").unwrap();
        let mut driver =
            Driver::capture(Some(&app.join("Cargo.toml")), &app, Selection::default()).unwrap();
        let frozen = driver.ws.inputs.source_snapshot().unwrap();
        assert!(frozen.contains(&dep));
        assert!(!frozen.contains(&base.join("unowned")));
        driver.target_dir = base.join("output");
        std::fs::remove_dir_all(&app).unwrap();
        std::fs::remove_dir_all(&dep).unwrap();
        for workers in [1, 8] {
            driver.sel.workers = workers;
            for command in [Command::Check, Command::Build, Command::Run] {
                assert_eq!(driver.run(command).unwrap(), 0);
            }
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
