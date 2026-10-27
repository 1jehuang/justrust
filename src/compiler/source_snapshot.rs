//! Immutable, caller-owned source namespace for closed native compilations.
//!
//! This is an input value, not a claim that a mutable host directory was observed
//! atomically. Callers supply the entire namespace at once. Missing entries stay
//! missing, and neither path resolution nor reads fall back to the host.
use super::{SourceReader, source_inputs::InputDomain};
use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_ENTRIES: usize = 4096;
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct SourceSnapshot {
    root: PathBuf,
    files: BTreeMap<PathBuf, Arc<[u8]>>,
    directories: BTreeSet<PathBuf>,
    digest: [u8; 32],
}
impl crate::engine::Fact for SourceSnapshot {}

impl SourceSnapshot {
    /// Create a closed namespace from owned bytes. Paths are relative to `root`
    /// or absolute beneath it. Symlinks, devices and ambient filesystem reads
    /// have no representation. Duplicate or file/directory-conflicting paths
    /// reject rather than depending on insertion order.
    pub fn new(
        root: PathBuf,
        files: impl IntoIterator<Item = (PathBuf, Vec<u8>)>,
    ) -> Result<Arc<Self>, String> {
        if !root.is_absolute() {
            return Err("source snapshot root must be absolute".into());
        }
        let root = normalize(&root).map_err(|e| e.to_string())?;
        if root == Path::new("/") || root.starts_with(super::source_prelude::PRELUDE_DIR) {
            return Err("source snapshot root must be a private non-root namespace".into());
        }
        let mut snapshot = Self {
            root: root.clone(),
            files: BTreeMap::new(),
            directories: BTreeSet::from([root]),
            digest: [0; 32],
        };
        let mut remaining = MAX_BYTES;
        for (path, bytes) in files {
            let path = snapshot.path(&path).map_err(|e| e.to_string())?;
            let cost = bytes.len().saturating_add(path.as_os_str().len());
            if snapshot.files.len() >= MAX_ENTRIES || cost > remaining {
                return Err("source snapshot input limit exceeded".into());
            }
            remaining -= cost;
            if snapshot.directories.contains(&path)
                || snapshot.files.insert(path.clone(), bytes.into()).is_some()
            {
                return Err("duplicate or conflicting source snapshot path".into());
            }
            let mut parent = path.parent();
            while let Some(path) = parent.filter(|p| p.starts_with(&snapshot.root)) {
                if snapshot.files.contains_key(path) {
                    return Err("source snapshot file is also a directory".into());
                }
                if snapshot.directories.insert(path.to_owned()) {
                    let cost = path.as_os_str().len();
                    if snapshot.directories.len() >= MAX_ENTRIES || cost > remaining {
                        return Err("source snapshot directory limit exceeded".into());
                    }
                    remaining -= cost;
                }
                parent = path.parent();
            }
        }
        let mut hash = blake3::Hasher::new();
        hash.update(b"justrust-immutable-source-namespace-v1");
        frame(&mut hash, snapshot.root.as_os_str().as_encoded_bytes());
        hash.update(&(snapshot.files.len() as u64).to_le_bytes());
        for (path, bytes) in &snapshot.files {
            frame(&mut hash, path.as_os_str().as_encoded_bytes());
            frame(&mut hash, bytes);
        }
        snapshot.digest = *hash.finalize().as_bytes();
        Ok(Arc::new(snapshot))
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn path(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normalize(&if path.is_absolute() {
            path.to_owned()
        } else {
            self.root.join(path)
        })?;
        if !path.starts_with(&self.root) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "path escapes the immutable source snapshot",
            ));
        }
        Ok(path)
    }

    pub(super) fn reader(&self) -> SnapshotReader<'_> {
        SnapshotReader(self)
    }
}

fn frame(hash: &mut blake3::Hasher, bytes: &[u8]) {
    hash.update(&(bytes.len() as u64).to_le_bytes());
    hash.update(bytes);
}

fn normalize(path: &Path) -> io::Result<PathBuf> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::RootDir => normalized.push(component),
            Component::Normal(value) => normalized.push(value),
            Component::CurDir => {}
            Component::ParentDir if normalized.file_name().is_some() => {
                normalized.pop();
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "invalid source snapshot path",
                ));
            }
        }
    }
    Ok(normalized)
}

pub(super) struct SnapshotReader<'a>(&'a SourceSnapshot);
impl SourceReader for SnapshotReader<'_> {
    fn input_domain(&self) -> InputDomain {
        InputDomain::Snapshot
    }
    fn canonicalize(&mut self, path: &Path) -> io::Result<PathBuf> {
        let path = self.0.path(path)?;
        if self.0.files.contains_key(&path) || self.0.directories.contains(&path) {
            Ok(path)
        } else {
            Err(io::Error::new(
                io::ErrorKind::NotFound,
                "snapshot path is absent",
            ))
        }
    }
    fn is_file(&mut self, path: &Path) -> io::Result<bool> {
        let path = self.canonicalize(path)?;
        Ok(self.0.files.contains_key(&path))
    }
    fn exists(&mut self, path: &Path) -> io::Result<bool> {
        let path = self.0.path(path)?;
        Ok(self.0.files.contains_key(&path) || self.0.directories.contains(&path))
    }
    fn read(&mut self, path: &Path, limit: usize) -> io::Result<Vec<u8>> {
        let path = self.0.path(path)?;
        let bytes =
            self.0.files.get(&path).ok_or_else(|| {
                io::Error::new(io::ErrorKind::NotFound, "snapshot file is absent")
            })?;
        Ok(bytes[..bytes.len().min(limit)].to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn snapshot(files: &[(&str, &str)]) -> Arc<SourceSnapshot> {
        SourceSnapshot::new(
            "/frozen".into(),
            files
                .iter()
                .map(|(p, b)| (PathBuf::from(p), b.as_bytes().to_vec())),
        )
        .unwrap()
    }
    #[test]
    fn snapshot_native_builds_are_independent_of_host_mutation() {
        use crate::compiler::{self, source::SourceOptions};
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("jr-frozen-native-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let source = "mod child; fn main() { assert_eq!(child::value(), 42); }";
        let child = "pub fn value() -> i32 { 42 }";
        std::fs::write(root.join("main.rs"), source).unwrap();
        std::fs::write(root.join("child.rs"), child).unwrap();
        let stock = root.join("stock");
        let status = std::process::Command::new(crate::compiler::stock_rustc::stock_rustc())
            .arg("--edition=2024")
            .arg(root.join("main.rs"))
            .arg("-o")
            .arg(&stock)
            .status()
            .unwrap();
        assert!(status.success());
        let expected = std::process::Command::new(&stock).output().unwrap();
        let frozen = SourceSnapshot::new(
            root.clone(),
            [
                ("main.rs".into(), source.as_bytes().to_vec()),
                ("child.rs".into(), child.as_bytes().to_vec()),
            ],
        )
        .unwrap();
        // The native graph must not read any of these replacement bytes, nor
        // the new ambiguous child/mod.rs candidate absent from its namespace.
        std::fs::write(root.join("main.rs"), "not Rust").unwrap();
        std::fs::remove_file(root.join("child.rs")).unwrap();
        std::fs::create_dir(root.join("child")).unwrap();
        std::fs::write(root.join("child/mod.rs"), "not Rust").unwrap();
        for workers in [1, 8] {
            for check_only in [true, false] {
                let options = SourceOptions {
                    source_snapshot: Some(frozen.clone()),
                    ..Default::default()
                };
                let compiled = compiler::compile_file_observed_with_options(
                    root.join("main.rs"),
                    workers,
                    check_only,
                    options,
                )
                .unwrap();
                assert_eq!(compiled.executable.is_none(), check_only);
                assert!(compiled.report.capture_complete);
                if check_only {
                    assert!(
                        compiled
                            .report
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| !matches!(
                                key.kind,
                                compiler::OBJECT | compiler::LOWER
                            ))
                    );
                }
                if let Some(bytes) = compiled.executable {
                    let executable = root.join(format!("native-{workers}"));
                    std::fs::write(&executable, bytes).unwrap();
                    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    let actual = std::process::Command::new(executable).output().unwrap();
                    assert_eq!(actual.status.code(), expected.status.code());
                    assert_eq!(actual.stdout, expected.stdout);
                    assert_eq!(actual.stderr, expected.stderr);
                }
            }
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn snapshot_rejects_missing_inputs_bad_unused_items_and_external_linking() {
        use crate::compiler::{self, source::SourceOptions};
        for workers in [1, 8] {
            for check_only in [true, false] {
                for source in [
                    "mod missing; fn main() {}",
                    "fn unused() -> i32 { true } fn main() {}",
                    "#[path=\"/etc/passwd\"] mod escaped; fn main() {}",
                ] {
                    let options = SourceOptions {
                        source_snapshot: Some(snapshot(&[("main.rs", source)])),
                        ..Default::default()
                    };
                    assert!(
                        compiler::compile_file_observed_with_options(
                            "/frozen/main.rs".into(),
                            workers,
                            check_only,
                            options
                        )
                        .is_err()
                    );
                }
            }
            let options = SourceOptions {
                source_snapshot: Some(snapshot(&[("main.rs", "fn main() {}")])),
                native_link: compiler::c_abi::elf::LinkOptions {
                    libraries: vec!["c".into()],
                    ..Default::default()
                },
                ..Default::default()
            };
            let failure = match compiler::compile_file_observed_with_options(
                "/frozen/main.rs".into(),
                workers,
                false,
                options,
            ) {
                Ok(_) => panic!("uncontrolled linker was admitted"),
                Err(failure) => failure,
            };
            assert!(
                failure.to_string().contains("uncontrolled native linker"),
                "{failure}"
            );
        }
    }

    #[test]
    fn snapshot_receipts_reject_changed_options_and_forged_reader_domains() {
        use crate::compiler::{expansion_budget::ExpansionBudget, source};
        let frozen = snapshot(&[("main.rs", "fn main() {}")]);
        let mut options = source::SourceOptions {
            source_snapshot: Some(frozen),
            prelude: false,
            ..Default::default()
        };
        let unit = source::prepare_root_unit(
            Path::new("/frozen/main.rs"),
            &options,
            &ExpansionBudget::default(),
        )
        .unwrap();
        assert!(unit.inputs.matches_current_source(&options));
        assert!(unit.inputs.validate_snapshot(&options).is_ok());
        options.source_snapshot = Some(snapshot(&[("main.rs", "fn main() { let x = 1; }")]));
        assert!(!unit.inputs.matches_current_source(&options));
        assert!(unit.inputs.validate_snapshot(&options).is_err());
        let untracked = source::source_inputs::SourceInputs::untracked();
        assert!(untracked.validate_snapshot(&options).is_err());
        options.project_inputs = Some(crate::project::inputs::DiscoveryInputs::untracked());
        assert!(
            source::prepare_root_unit(
                Path::new("/frozen/main.rs"),
                &options,
                &ExpansionBudget::default()
            )
            .is_err()
        );
    }

    #[test]
    fn namespace_is_closed_order_independent_and_bounded() {
        let first = snapshot(&[("main.rs", "mod child;"), ("child.rs", "")]);
        let second = snapshot(&[("child.rs", ""), ("./main.rs", "mod child;")]);
        assert_eq!(first.digest(), second.digest());
        assert_ne!(first.digest(), snapshot(&[("main.rs", "changed")]).digest());
        let mut reader = first.reader();
        assert!(!reader.exists(Path::new("absent")).unwrap());
        assert!(reader.read(Path::new("/etc/passwd"), 100).is_err());
        assert!(reader.exists(Path::new("../escape")).is_err());
        assert_eq!(reader.read(Path::new("child.rs"), 100).unwrap(), b"");
        assert!(SourceSnapshot::new("relative".into(), []).is_err());
        assert!(SourceSnapshot::new("/".into(), []).is_err());
        for files in [
            vec![("a", ""), ("a", "")],
            vec![("a/b", ""), ("a", "")],
            vec![("a", ""), ("a/b", "")],
        ] {
            assert!(
                SourceSnapshot::new(
                    "/frozen".into(),
                    files
                        .into_iter()
                        .map(|(p, b)| (p.into(), b.as_bytes().to_vec()))
                )
                .is_err()
            );
        }
        assert!(
            SourceSnapshot::new("/frozen".into(), [("big".into(), vec![0; MAX_BYTES + 1])])
                .is_err()
        );
    }
}
