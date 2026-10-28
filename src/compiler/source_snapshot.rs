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
    roots: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, Arc<[u8]>>,
    directories: BTreeSet<PathBuf>,
    digest: [u8; 32],
}
impl crate::engine::Fact for SourceSnapshot {}

impl SourceSnapshot {
    /// Acquire owned directory bytes with bounded, repeated observations and
    /// mutation checks. This is not a filesystem-wide atomic snapshot, and
    /// cannot authorize artifact sharing. Excluded output/cache paths stay
    /// absent for every subsequent compiler read.
    pub fn capture_directory(root: &Path, excluded_output: &Path) -> Result<Arc<Self>, String> {
        crate::project::capture::directory(root, excluded_output).map_err(|error| error.0)
    }

    /// Capture explicitly authorized source directories under one aggregate
    /// acquisition budget. No common ancestor or ambient siblings are admitted.
    pub fn capture_directories(
        root: &Path,
        additional: &[PathBuf],
        excluded_output: &Path,
    ) -> Result<Arc<Self>, String> {
        crate::project::capture::directories(root, additional, excluded_output)
            .map_err(|error| error.0)
    }

    /// Create a closed namespace from owned bytes. Paths are relative to `root`
    /// or absolute beneath it. Symlinks, devices and ambient filesystem reads
    /// have no representation. Duplicate or file/directory-conflicting paths
    /// reject rather than depending on insertion order.
    pub fn new(
        root: PathBuf,
        files: impl IntoIterator<Item = (PathBuf, Vec<u8>)>,
    ) -> Result<Arc<Self>, String> {
        Self::with_roots(root, &[], files)
    }

    pub(crate) fn with_roots(
        root: PathBuf,
        additional: &[PathBuf],
        files: impl IntoIterator<Item = (PathBuf, Vec<u8>)>,
    ) -> Result<Arc<Self>, String> {
        let roots = Self::normalized_roots(&root, additional)?;
        let root = normalize(&root).map_err(|e| e.to_string())?;
        let mut remaining = MAX_BYTES;
        for path in &roots {
            remaining = remaining
                .checked_sub(path.as_os_str().len())
                .ok_or("source snapshot root limit exceeded")?;
        }
        let mut snapshot = Self {
            root,
            directories: roots.clone(),
            roots,
            files: BTreeMap::new(),
            digest: [0; 32],
        };
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
            while let Some(path) = parent.filter(|p| snapshot.contains(p)) {
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
        hash.update(b"justrust-immutable-source-namespace-v2");
        frame(&mut hash, snapshot.root.as_os_str().as_encoded_bytes());
        hash.update(&(snapshot.roots.len() as u64).to_le_bytes());
        for root in &snapshot.roots {
            frame(&mut hash, root.as_os_str().as_encoded_bytes());
        }
        hash.update(&(snapshot.files.len() as u64).to_le_bytes());
        for (path, bytes) in &snapshot.files {
            frame(&mut hash, path.as_os_str().as_encoded_bytes());
            frame(&mut hash, bytes);
        }
        snapshot.digest = *hash.finalize().as_bytes();
        Ok(Arc::new(snapshot))
    }

    pub(crate) fn normalized_roots(
        root: &Path,
        additional: &[PathBuf],
    ) -> Result<BTreeSet<PathBuf>, String> {
        if additional.len() >= 64 {
            return Err("source snapshot root limit exceeded".into());
        }
        let mut roots = BTreeSet::new();
        for root in std::iter::once(root).chain(additional.iter().map(PathBuf::as_path)) {
            if !root.is_absolute() {
                return Err("source snapshot root must be absolute".into());
            }
            let root = normalize(root).map_err(|e| e.to_string())?;
            if root == Path::new("/") || root.starts_with(super::source_prelude::PRELUDE_DIR) {
                return Err("source snapshot root must be a private non-root namespace".into());
            }
            if !roots
                .iter()
                .any(|parent: &PathBuf| root.starts_with(parent))
            {
                roots.retain(|child: &PathBuf| !child.starts_with(&root));
                roots.insert(root);
            }
        }
        Ok(roots)
    }

    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub(crate) fn contains(&self, path: &Path) -> bool {
        normalize(path).is_ok_and(|path| self.roots.iter().any(|root| path.starts_with(root)))
    }

    fn path(&self, path: &Path) -> io::Result<PathBuf> {
        let path = normalize(&if path.is_absolute() {
            path.to_owned()
        } else {
            self.root.join(path)
        })?;
        if !self.contains(&path) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "path escapes the immutable source snapshot",
            ));
        }
        Ok(path)
    }

    pub(crate) fn reader(&self) -> SnapshotReader<'_> {
        SnapshotReader(self)
    }

    pub(crate) fn paths(&self) -> impl Iterator<Item = &Path> {
        self.files.keys().map(PathBuf::as_path)
    }

    pub(crate) fn read_dir(&self, path: &Path) -> io::Result<Vec<std::ffi::OsString>> {
        let path = self.path(path)?;
        if !self.directories.contains(&path) {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "snapshot directory is absent",
            ));
        }
        Ok(self
            .files
            .keys()
            .chain(self.directories.iter())
            .filter(|entry| entry.parent() == Some(path.as_path()))
            .filter_map(|entry| entry.file_name().map(ToOwned::to_owned))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect())
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

pub(crate) struct SnapshotReader<'a>(&'a SourceSnapshot);
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
    #[test]
    fn multi_root_identity_binds_authorization_without_admitting_common_ancestors() {
        let app = PathBuf::from("/frozen/app");
        let dep = PathBuf::from("/frozen/dep");
        let extra = PathBuf::from("/frozen/extra");
        let files = || {
            vec![
                (app.join("main.rs"), b"fn main() {}".to_vec()),
                (dep.join("lib.rs"), vec![]),
            ]
        };
        let first = SourceSnapshot::with_roots(app.clone(), &[dep.clone(), extra.clone()], files())
            .unwrap();
        let reordered = SourceSnapshot::with_roots(
            app.clone(),
            &[extra.clone(), dep.clone(), dep.clone()],
            files(),
        )
        .unwrap();
        assert_eq!(first.digest(), reordered.digest());
        let fewer =
            SourceSnapshot::with_roots(app.clone(), std::slice::from_ref(&dep), files()).unwrap();
        assert_ne!(first.digest(), fewer.digest());
        assert!(first.reader().canonicalize(&extra).is_ok());
        assert!(fewer.reader().canonicalize(&extra).is_err());
        assert!(first.reader().canonicalize(Path::new("/frozen")).is_err());
        assert!(
            first
                .reader()
                .read(Path::new("/frozen/unowned"), 100)
                .is_err()
        );
        assert!(first.reader().read(Path::new("../dep/lib.rs"), 100).is_ok());
        assert!(
            first
                .reader()
                .read(Path::new("../unowned/lib.rs"), 100)
                .is_err()
        );
        assert!(
            SourceSnapshot::with_roots(
                app.clone(),
                std::slice::from_ref(&dep),
                [(PathBuf::from("/frozen/unowned"), vec![])]
            )
            .is_err()
        );
        let nested =
            SourceSnapshot::with_roots(app.clone(), &[dep.clone(), app.join("nested")], files())
                .unwrap();
        assert_eq!(nested.digest(), fewer.digest());
        assert!(SourceSnapshot::with_roots(app.clone(), &[PathBuf::from("/")], files()).is_err());
        assert!(
            SourceSnapshot::with_roots(app.clone(), &[PathBuf::from("relative")], files()).is_err()
        );
    }

    #[test]
    fn frozen_project_discovery_drives_check_build_run_without_host_reads() {
        use crate::project::{
            driver::{Command, Driver, Selection},
            manifest,
        };
        let root = std::env::temp_dir().join(format!("jr-frozen-project-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("dep/src")).unwrap();
        let files = [
            (
                "Cargo.toml",
                "[package]\nname='frozen_app'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={path='dep'}\n",
            ),
            ("src/main.rs", "fn main() { assert_eq!(dep::value(), 42); }"),
            (
                "dep/Cargo.toml",
                "[package]\nname='dep'\nversion='0.1.0'\nedition='2024'\n",
            ),
            ("dep/src/lib.rs", "pub fn value() -> i32 { 42 }"),
        ];
        for (path, bytes) in files {
            std::fs::write(root.join(path), bytes).unwrap();
        }
        let stock = crate::compiler::stock_rustc::stock_rustc();
        assert!(
            std::process::Command::new(&stock)
                .args([
                    "--edition=2024",
                    "--crate-name",
                    "dep",
                    "--crate-type",
                    "rlib"
                ])
                .arg(root.join("dep/src/lib.rs"))
                .arg("-o")
                .arg(root.join("libdep.rlib"))
                .status()
                .unwrap()
                .success()
        );
        assert!(
            std::process::Command::new(&stock)
                .arg("--edition=2024")
                .arg(root.join("src/main.rs"))
                .arg("--extern")
                .arg(format!("dep={}", root.join("libdep.rlib").display()))
                .arg("-o")
                .arg(root.join("stock"))
                .status()
                .unwrap()
                .success()
        );
        assert!(
            std::process::Command::new(root.join("stock"))
                .status()
                .unwrap()
                .success()
        );
        let frozen = SourceSnapshot::new(
            root.clone(),
            files
                .into_iter()
                .map(|(p, b)| (p.into(), b.as_bytes().to_vec())),
        )
        .unwrap();
        let environment = BTreeMap::from([(
            "CARGO_TARGET_DIR".to_owned(),
            root.join("output").into_os_string(),
        )]);
        let first = manifest::load_snapshot(
            frozen.clone(),
            &root.join("Cargo.toml"),
            environment.clone(),
        )
        .unwrap();
        assert!(first.inputs.identity().is_some());
        assert_eq!(first.packages.len(), 2);
        for (path, _) in files {
            std::fs::remove_file(root.join(path)).unwrap();
        }
        std::fs::write(root.join("Cargo.toml"), "not TOML").unwrap();
        std::fs::write(root.join("src/main.rs"), "not Rust").unwrap();
        let after = manifest::load_snapshot(
            frozen.clone(),
            &root.join("Cargo.toml"),
            environment.clone(),
        )
        .unwrap();
        assert_eq!(first.inputs.identity(), after.inputs.identity());
        assert!(first.inputs.matches_current());
        for workers in [1, 8] {
            let driver = Driver::from_snapshot(
                frozen.clone(),
                &root.join("Cargo.toml"),
                environment.clone(),
                Selection {
                    workers,
                    ..Default::default()
                },
            )
            .unwrap();
            for command in [Command::Check, Command::Build, Command::Run] {
                assert_eq!(driver.run(command).unwrap(), 0);
            }
        }
        let changed = SourceSnapshot::new(
            root.clone(),
            files.into_iter().map(|(p, b)| {
                (
                    p.into(),
                    if p == "dep/src/lib.rs" {
                        b"pub fn value() -> i32 { 7 }".to_vec()
                    } else {
                        b.as_bytes().to_vec()
                    },
                )
            }),
        )
        .unwrap();
        let options = crate::compiler::source::SourceOptions {
            project_inputs: Some(first.inputs.clone()),
            source_snapshot: Some(changed),
            ..Default::default()
        };
        assert!(
            crate::compiler::compile_file_observed_with_options(
                root.join("src/main.rs"),
                1,
                true,
                options
            )
            .is_err()
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn frozen_project_rejects_configs_tools_and_missing_manifests() {
        use crate::project::manifest;
        let root = PathBuf::from("/frozen-project-policy");
        let base = [
            (
                "Cargo.toml",
                "[package]\nname='app'\nversion='0.1.0'\nedition='2024'\n",
            ),
            ("src/main.rs", "fn main() {}"),
        ];
        let make = |extra: Option<(&str, &str)>| {
            SourceSnapshot::new(
                root.clone(),
                base.into_iter()
                    .chain(extra)
                    .map(|(p, b)| (p.into(), b.as_bytes().to_vec())),
            )
            .unwrap()
        };
        for extra in [
            (".cargo/config.toml", "[build]\nrustflags=['bad']"),
            ("build.rs", "fn main() {}"),
        ] {
            assert!(
                manifest::load_snapshot(
                    make(Some(extra)),
                    &root.join("Cargo.toml"),
                    BTreeMap::new()
                )
                .is_err()
            );
        }
        assert!(
            manifest::load_snapshot(
                make(None),
                &root.join("missing/Cargo.toml"),
                BTreeMap::new()
            )
            .is_err()
        );
        assert!(
            manifest::load_snapshot(
                make(None),
                &root.join("Cargo.toml"),
                BTreeMap::from([("RUSTFLAGS".into(), "-C opt-level=3".into())])
            )
            .is_err()
        );
        let registry = SourceSnapshot::new(
            root.clone(),
            [
                (
                    "Cargo.toml".into(),
                    b"[package]\nname='app'\nversion='0.1.0'\n[dependencies]\ndep='1'\n".to_vec(),
                ),
                ("src/main.rs".into(), b"fn main() {}".to_vec()),
            ],
        )
        .unwrap();
        // Reject before probing registry caches or invoking archive tools.
        assert!(
            manifest::load_snapshot(registry, &root.join("Cargo.toml"), BTreeMap::new()).is_err()
        );
    }
    #[test]
    fn frozen_workspace_globs_and_environment_are_snapshot_bound() {
        use crate::project::manifest;
        let root = PathBuf::from("/frozen-workspace-globs");
        let files = [
            (
                "Cargo.toml",
                "[workspace]\nmembers=['crates/*']\nresolver='2'\n",
            ),
            (
                "crates/app/Cargo.toml",
                "[package]\nname='app'\nversion='0.1.0'\nedition='2024'\n",
            ),
            ("crates/app/src/main.rs", "fn main() {}"),
            ("crates/app/src/bin/extra.rs", "fn main() {}"),
        ];
        let frozen = SourceSnapshot::new(
            root.clone(),
            files
                .into_iter()
                .map(|(p, b)| (p.into(), b.as_bytes().to_vec())),
        )
        .unwrap();
        let first =
            manifest::load_snapshot(frozen.clone(), &root.join("Cargo.toml"), BTreeMap::new())
                .unwrap();
        assert_eq!(first.members.len(), 1);
        assert_eq!(first.packages[0].targets.len(), 2);
        let changed_env = manifest::load_snapshot(
            frozen.clone(),
            &root.join("Cargo.toml"),
            BTreeMap::from([("CARGO_TARGET_DIR".into(), "/another-output".into())]),
        )
        .unwrap();
        assert_ne!(first.inputs.identity(), changed_env.inputs.identity());
        let missing = SourceSnapshot::new(
            root.clone(),
            files
                .into_iter()
                .filter(|(p, _)| *p != "crates/app/Cargo.toml")
                .map(|(p, b)| {
                    let bytes = if p == "Cargo.toml" {
                        b"[workspace]\nmembers=['crates/app']\nresolver='2'\n".to_vec()
                    } else {
                        b.as_bytes().to_vec()
                    };
                    (p.into(), bytes)
                }),
        )
        .unwrap();
        assert!(
            manifest::load_snapshot(missing, &root.join("Cargo.toml"), BTreeMap::new()).is_err()
        );
    }
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
    fn snapshot_rejects_missing_inputs_and_bad_unused_items_but_admits_controlled_linking() {
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
            let linked = compiler::compile_file_observed_with_options(
                "/frozen/main.rs".into(),
                workers,
                false,
                options,
            )
            .unwrap();
            assert!(
                linked
                    .executable
                    .as_ref()
                    .is_some_and(|bytes| bytes.starts_with(b"\x7fELF"))
            );
            assert!(linked.report.capture_complete);
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
