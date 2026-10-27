//! Bounded observations from native project discovery. No cache authorization.
//! Each loader owns its recorder. Only the immutable receipt crosses into the
//! source graph; no mutable recorder or filesystem handle is published.
use crate::compiler::source::{SourceReader, SourceSnapshot};
use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::Arc;

const MAX_ENTRIES: usize = 65_536;
const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Observed {
    Canonical(Result<PathBuf, io::ErrorKind>),
    FileKind(Result<bool, io::ErrorKind>),
    File(Result<(usize, [u8; 32]), io::ErrorKind>),
    Directory(Result<Vec<OsString>, io::ErrorKind>),
    Environment(Option<OsString>),
}
#[derive(Clone, Debug)]
struct Observation {
    name: PathBuf,
    value: Observed,
}

/// Discovery evidence is separate from whole-build completeness. Registry
/// tools, registry planning IO and atomic snapshots remain outside this scope.
#[derive(Clone, Debug)]
pub struct DiscoveryInputs {
    snapshot: Option<Arc<SourceSnapshot>>,
    environment: std::collections::BTreeMap<String, Option<OsString>>,
    path_packages: std::collections::BTreeMap<PathBuf, usize>,
    registry_packages:
        std::collections::BTreeMap<super::manifest::LockedRegistryPackage, RegistryBinding>,
    observations: Vec<Observation>,
    complete: bool,
    uncontrolled: bool,
    identity: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RegistryBinding {
    pub(crate) index: usize,
    pub(crate) root: PathBuf,
    pub(crate) archive: PathBuf,
}

impl crate::engine::Fact for DiscoveryInputs {}
impl DiscoveryInputs {
    pub(crate) fn untracked() -> Arc<Self> {
        Arc::new(Self {
            snapshot: None,
            environment: std::collections::BTreeMap::new(),
            path_packages: std::collections::BTreeMap::new(),
            registry_packages: std::collections::BTreeMap::new(),
            observations: Vec::new(),
            complete: false,
            uncontrolled: true,
            identity: [0; 32],
        })
    }
    pub(crate) fn path_package(&self, path: &Path) -> Option<usize> {
        self.path_packages.get(path).copied()
    }
    pub(crate) fn registry_package(
        &self,
        package: &super::manifest::LockedRegistryPackage,
    ) -> Option<&RegistryBinding> {
        self.registry_packages.get(package)
    }
    pub fn observation_count(&self) -> usize {
        self.observations.len()
    }
    pub fn source_snapshot(&self) -> Option<&Arc<SourceSnapshot>> {
        self.snapshot.as_ref()
    }
    pub fn identity(&self) -> Option<&[u8; 32]> {
        (self.complete && !self.uncontrolled).then_some(&self.identity)
    }
    pub fn environment(&self, name: &str) -> Option<&OsStr> {
        self.environment
            .get(name)
            .and_then(|value| value.as_deref())
    }
    /// Revalidate discovery only. This is not a race-free filesystem snapshot.
    pub fn matches_current(&self) -> bool {
        if self.snapshot.is_some() {
            // Both the namespace and environment are owned immutable inputs.
            return self.complete && !self.uncontrolled;
        }
        self.complete
            && !self.uncontrolled
            && self.observations.iter().all(|o| {
                let actual = match &o.value {
                    Observed::Canonical(_) => {
                        Observed::Canonical(std::fs::canonicalize(&o.name).map_err(|e| e.kind()))
                    }
                    Observed::FileKind(_) => Observed::FileKind(
                        std::fs::metadata(&o.name)
                            .map(|m| m.is_file())
                            .map_err(|e| e.kind()),
                    ),
                    Observed::File(_) => Observed::File(
                        read_bounded(&o.name)
                            .map(|v| (v.len(), *blake3::hash(&v).as_bytes()))
                            .map_err(|e| e.kind()),
                    ),
                    Observed::Directory(_) => {
                        Observed::Directory(list_bounded(&o.name).map_err(|e| e.kind()))
                    }
                    Observed::Environment(_) => Observed::Environment(std::env::var_os(&o.name)),
                };
                actual == o.value
            })
    }
}

pub(crate) struct Recorder {
    snapshot: Option<Arc<SourceSnapshot>>,
    environment: std::collections::BTreeMap<String, Option<OsString>>,
    path_packages: std::collections::BTreeMap<PathBuf, usize>,
    registry_packages:
        std::collections::BTreeMap<super::manifest::LockedRegistryPackage, RegistryBinding>,
    observations: Vec<Observation>,
    cwd: Option<PathBuf>,
    complete: bool,
    uncontrolled: bool,
    remaining: usize,
    binding_remaining: usize,
    identity: blake3::Hasher,
}
impl Recorder {
    pub(crate) fn new() -> Self {
        let cwd = std::env::current_dir().ok();
        let mut identity = blake3::Hasher::new();
        identity.update(b"justrust-project-discovery-v2");
        Self {
            snapshot: None,
            environment: std::collections::BTreeMap::new(),
            path_packages: std::collections::BTreeMap::new(),
            registry_packages: std::collections::BTreeMap::new(),
            observations: Vec::new(),
            complete: cwd.is_some(),
            cwd,
            uncontrolled: false,
            remaining: MAX_BYTES,
            binding_remaining: MAX_BYTES,
            identity,
        }
    }
    pub(crate) fn frozen(
        snapshot: Arc<SourceSnapshot>,
        environment: std::collections::BTreeMap<String, OsString>,
    ) -> Result<Self, &'static str> {
        let mut size = 0usize;
        for (name, value) in &environment {
            if !matches!(name.as_str(), "CARGO_TARGET_DIR" | "CARGO_HOME" | "HOME") {
                return Err(
                    "unsupported: immutable project environment contains an uncontrolled setting",
                );
            }
            size = size.saturating_add(name.len()).saturating_add(value.len());
        }
        if size > MAX_BYTES {
            return Err("immutable project environment exceeds input limit");
        }
        let mut identity = blake3::Hasher::new();
        identity.update(b"justrust-frozen-project-discovery-v1");
        identity.update(snapshot.digest());
        Ok(Self {
            cwd: Some(snapshot.root().to_owned()),
            snapshot: Some(snapshot),
            environment: environment.into_iter().map(|(k, v)| (k, Some(v))).collect(),
            path_packages: Default::default(),
            registry_packages: Default::default(),
            observations: Vec::new(),
            complete: true,
            uncontrolled: false,
            remaining: MAX_BYTES,
            binding_remaining: MAX_BYTES,
            identity,
        })
    }
    pub(crate) fn is_frozen(&self) -> bool {
        self.snapshot.is_some()
    }
    pub(crate) fn within_namespace(&self, path: &Path) -> bool {
        self.snapshot
            .as_ref()
            .is_none_or(|snapshot| snapshot.contains(path))
    }
    fn record(&mut self, path: &Path, value: Observed) {
        if !self.complete {
            return;
        }
        let name = if matches!(value, Observed::Environment(_)) || path.is_absolute() {
            path.to_owned()
        } else {
            self.cwd.as_ref().unwrap().join(path)
        };
        let mut encoded = Vec::new();
        fn bytes(out: &mut Vec<u8>, v: &[u8]) {
            out.extend_from_slice(&(v.len() as u64).to_le_bytes());
            out.extend_from_slice(v);
        }
        bytes(&mut encoded, name.as_os_str().as_encoded_bytes());
        // Debug is deliberately NOT an identity format. Every variant and result
        // has an explicit tag, and every byte string is length framed.
        match &value {
            Observed::Canonical(r) => {
                encoded.push(0);
                match r {
                    Ok(p) => {
                        encoded.push(0);
                        bytes(&mut encoded, p.as_os_str().as_encoded_bytes());
                    }
                    Err(e) => {
                        encoded.push(1);
                        encode_error(&mut encoded, *e);
                    }
                }
            }
            Observed::FileKind(r) => {
                encoded.push(1);
                match r {
                    Ok(b) => encoded.extend_from_slice(&[0, u8::from(*b)]),
                    Err(e) => {
                        encoded.push(1);
                        encode_error(&mut encoded, *e);
                    }
                }
            }
            Observed::File(r) => {
                encoded.push(2);
                match r {
                    Ok((len, hash)) => {
                        encoded.push(0);
                        encoded.extend_from_slice(&(*len as u64).to_le_bytes());
                        encoded.extend_from_slice(hash);
                    }
                    Err(e) => {
                        encoded.push(1);
                        encode_error(&mut encoded, *e);
                    }
                }
            }
            Observed::Directory(r) => {
                encoded.push(3);
                match r {
                    Ok(names) => {
                        encoded.push(0);
                        encoded.extend_from_slice(&(names.len() as u64).to_le_bytes());
                        for n in names {
                            bytes(&mut encoded, n.as_encoded_bytes());
                        }
                    }
                    Err(e) => {
                        encoded.push(1);
                        encode_error(&mut encoded, *e);
                    }
                }
            }
            Observed::Environment(v) => {
                encoded.push(4);
                match v {
                    Some(v) => {
                        encoded.push(1);
                        bytes(&mut encoded, v.as_encoded_bytes());
                    }
                    None => encoded.push(0),
                }
            }
        }
        if self
            .observations
            .len()
            .saturating_add(self.path_packages.len())
            >= MAX_ENTRIES
            || encoded.len() > self.remaining
        {
            self.complete = false;
            return;
        }
        self.remaining -= encoded.len();
        self.identity.update(&encoded);
        self.observations.push(Observation { name, value });
    }
    fn error(&mut self, error: io::ErrorKind) {
        // Absence is a normal negative dependency. Other failures can be
        // suppressed by the historical loader but never authorize revalidation.
        if !matches!(
            error,
            io::ErrorKind::NotFound | io::ErrorKind::NotADirectory
        ) {
            self.complete = false;
        }
    }
    pub(crate) fn canonicalize(&mut self, path: &Path) -> io::Result<PathBuf> {
        let r = match &self.snapshot {
            Some(snapshot) => snapshot.reader().canonicalize(path),
            None => path.canonicalize(),
        };
        self.record(
            path,
            Observed::Canonical(r.as_ref().cloned().map_err(|e| e.kind())),
        );
        if let Err(e) = &r {
            self.error(e.kind());
        }
        r
    }
    pub(crate) fn is_file(&mut self, path: &Path) -> bool {
        let r = match &self.snapshot {
            Some(snapshot) => snapshot.reader().is_file(path),
            None => std::fs::metadata(path).map(|m| m.is_file()),
        };
        self.record(
            path,
            Observed::FileKind(r.as_ref().copied().map_err(|e| e.kind())),
        );
        if let Err(e) = &r {
            self.error(e.kind());
        }
        r.unwrap_or(false)
    }
    pub(crate) fn read_to_string(&mut self, path: &Path) -> io::Result<String> {
        let r = match &self.snapshot {
            Some(snapshot) => snapshot.reader().read(path, MAX_BYTES + 1),
            None => read_bounded(path),
        };
        self.record(
            path,
            Observed::File(
                r.as_ref()
                    .map(|v| (v.len(), *blake3::hash(v).as_bytes()))
                    .map_err(|e| e.kind()),
            ),
        );
        if let Err(e) = &r {
            self.error(e.kind());
        }
        String::from_utf8(r?).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
    }
    pub(crate) fn read_dir(&mut self, path: &Path) -> io::Result<Vec<PathBuf>> {
        let r = match &self.snapshot {
            Some(snapshot) => snapshot.read_dir(path),
            None => list_bounded(path),
        };
        self.record(
            path,
            Observed::Directory(r.as_ref().cloned().map_err(|e| e.kind())),
        );
        if let Err(e) = &r {
            self.error(e.kind());
        }
        r.map(|names| names.into_iter().map(|n| path.join(n)).collect())
    }
    pub(crate) fn env(&mut self, name: &str) -> Option<OsString> {
        let value = if self.snapshot.is_some() {
            self.environment.get(name).cloned().flatten()
        } else {
            std::env::var_os(name)
        };
        self.environment.insert(name.to_owned(), value.clone());
        self.record(Path::new(name), Observed::Environment(value.clone()));
        value
    }
    /// Freeze the package selected by an already-observed canonical path read.
    /// Exhausted observation recording disables identity without losing bindings.
    /// Exhausting the separately bounded binding storage rejects discovery.
    pub(crate) fn bind_path_package(
        &mut self,
        path: &Path,
        index: usize,
    ) -> Result<(), &'static str> {
        if let Some(previous) = self.path_packages.get(path) {
            if *previous != index {
                self.complete = false;
                return Err("changed during project discovery");
            }
            return Ok(());
        }
        let bytes = path.as_os_str().as_encoded_bytes();
        let cost = bytes.len().saturating_add(17);
        if self.path_packages.len() >= MAX_ENTRIES || cost > self.binding_remaining {
            self.complete = false;
            return Err("exceeded project discovery path identity limits");
        }
        self.binding_remaining -= cost;
        if self.complete {
            if self
                .observations
                .len()
                .saturating_add(self.path_packages.len())
                >= MAX_ENTRIES
                || cost > self.remaining
            {
                self.complete = false;
            } else {
                self.remaining -= cost;
                self.identity.update(&[5]);
                self.identity.update(&(bytes.len() as u64).to_le_bytes());
                self.identity.update(bytes);
                self.identity.update(&(index as u64).to_le_bytes());
            }
        }
        self.path_packages.insert(path.to_owned(), index);
        Ok(())
    }
    /// Freeze an authenticated selection, not permission to reuse its mutable bytes.
    /// Registry inspection still invokes an uncontrolled external tool.
    pub(crate) fn bind_registry_package(
        &mut self,
        package: &super::manifest::LockedRegistryPackage,
        binding: RegistryBinding,
    ) -> Result<(), &'static str> {
        self.uncontrolled();
        if let Some(previous) = self.registry_packages.get(package) {
            return if previous == &binding {
                Ok(())
            } else {
                Err("registry selection changed during project discovery")
            };
        }
        let cost = package
            .name
            .len()
            .saturating_add(package.version.len())
            .saturating_add(package.source.len())
            .saturating_add(package.checksum.len())
            .saturating_add(binding.root.as_os_str().len())
            .saturating_add(binding.archive.as_os_str().len())
            .saturating_add(64);
        if self
            .registry_packages
            .len()
            .saturating_add(self.path_packages.len())
            >= MAX_ENTRIES
            || cost > self.binding_remaining
        {
            return Err("exceeded project discovery registry identity limits");
        }
        self.binding_remaining -= cost;
        self.registry_packages.insert(package.clone(), binding);
        Ok(())
    }
    pub(crate) fn uncontrolled(&mut self) {
        self.uncontrolled = true;
    }
    /// Legacy discovery does not interpret Cargo configuration. Observe both
    /// spellings, including absence, but never authorize reuse when one exists.
    pub(crate) fn observe_live_configuration(&mut self) {
        if self.is_frozen() {
            return;
        }
        let Some(cwd) = self.cwd.clone() else {
            self.complete = false;
            return;
        };
        let home = self.env("CARGO_HOME").map(PathBuf::from).or_else(|| {
            self.env("HOME")
                .map(|home| PathBuf::from(home).join(".cargo"))
        });
        let mut directories = std::collections::BTreeSet::new();
        for ancestor in cwd.ancestors() {
            if directories.len() >= MAX_ENTRIES / 2 {
                self.complete = false;
                return;
            }
            directories.insert(ancestor.join(".cargo"));
        }
        if let Some(home) = home {
            directories.insert(if home.is_absolute() {
                home
            } else {
                cwd.join(home)
            });
        } else {
            // Without a known Cargo home its configuration cannot be excluded.
            self.complete = false;
        }
        for directory in directories {
            for name in ["config", "config.toml"] {
                let path = directory.join(name);
                if self.canonicalize(&path).is_ok() {
                    self.uncontrolled();
                    if self.is_file(&path) {
                        let _ = self.read_to_string(&path);
                    }
                }
            }
        }
    }
    pub(crate) fn finish(self) -> Arc<DiscoveryInputs> {
        Arc::new(DiscoveryInputs {
            snapshot: self.snapshot,
            environment: self.environment,
            path_packages: self.path_packages,
            registry_packages: self.registry_packages,
            observations: self.observations,
            complete: self.complete,
            uncontrolled: self.uncontrolled,
            identity: *self.identity.finalize().as_bytes(),
        })
    }
}
fn encode_error(out: &mut Vec<u8>, kind: io::ErrorKind) {
    // Only ordinary negative-path observations are eligible. Other errors invalidate the
    // receipt, and their exact representation is deliberately not reusable.
    out.push(match kind {
        io::ErrorKind::NotFound => 0,
        io::ErrorKind::NotADirectory => 1,
        _ => 2,
    });
}
fn read_bounded(path: &Path) -> io::Result<Vec<u8>> {
    let file = std::fs::File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("discovery input must be a regular file"));
    }
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(io::Error::other(
            "project discovery input byte limit exceeded",
        ));
    }
    Ok(bytes)
}
fn list_bounded(path: &Path) -> io::Result<Vec<OsString>> {
    let mut names = Vec::new();
    let mut bytes = 0usize;
    for entry in std::fs::read_dir(path)? {
        let name = entry?.file_name();
        bytes = bytes.saturating_add(name.len());
        if names.len() >= MAX_ENTRIES || bytes > MAX_BYTES {
            return Err(io::Error::other(
                "project discovery directory limit exceeded",
            ));
        }
        names.push(name);
    }
    names.sort();
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::manifest;
    use std::sync::atomic::{AtomicUsize, Ordering};
    const MANIFEST: &str = "[package]\nname='tracked'\nversion='0.1.0'\nedition='2024'\n";
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let base = std::env::var_os("JCODE_SCRATCH_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(std::env::temp_dir);
            let root = base.join(format!(
                "jr-discovery-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        fn package(&self) {
            self.write("Cargo.toml", MANIFEST);
            self.write("src/main.rs", "fn main() {}\n");
        }
        fn load(&self) -> manifest::Workspace {
            manifest::load(&self.0.join("Cargo.toml")).unwrap()
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn live_configuration_appearance_invalidates_and_disables_identity() {
        let f = Fixture::new();
        f.package();
        let capture = || {
            let mut inputs = Recorder::new();
            inputs.cwd = Some(f.0.clone());
            inputs.observe_live_configuration();
            inputs.finish()
        };
        let absent = capture();
        assert!(absent.matches_current());
        for name in ["config", "config.toml"] {
            f.write(&format!(".cargo/{name}"), "[build]\njobs=2\n");
            assert!(!absent.matches_current());
            let present = capture();
            assert!(present.identity().is_none());
            assert!(!present.matches_current());
            std::fs::remove_file(f.0.join(".cargo").join(name)).unwrap();
            assert!(absent.matches_current());
        }
        std::fs::create_dir(f.0.join(".cargo/config.toml")).unwrap();
        assert!(capture().identity().is_none());
    }

    #[test]
    fn discovery_tracks_manifest_lock_absence_and_target_listings() {
        let f = Fixture::new();
        f.package();
        let original = f.load().inputs;
        assert!(original.matches_current());
        assert!(original.observation_count() > 10);
        f.write("Cargo.toml", &MANIFEST.replace("0.1.0", "0.1.1"));
        assert!(!original.matches_current());
        f.write("Cargo.toml", MANIFEST);
        assert!(original.matches_current());
        f.write("Cargo.lock", "version = 4\n");
        assert!(!original.matches_current());
        let locked = f.load().inputs;
        assert!(locked.matches_current());
        f.write("Cargo.lock", "version = 3\n");
        assert!(!locked.matches_current());
        std::fs::remove_file(f.0.join("Cargo.lock")).unwrap();
        assert!(original.matches_current());
        std::fs::create_dir_all(f.0.join("src/bin")).unwrap();
        assert!(!original.matches_current());
        let directory = f.load().inputs;
        assert!(directory.matches_current());
        f.write("src/bin/second.rs", "fn main() {}\n");
        assert!(!directory.matches_current());
        let added = f.load();
        assert_eq!(added.packages[0].targets.len(), 2);
        assert!(added.inputs.matches_current());
        std::fs::remove_file(f.0.join("src/bin/second.rs")).unwrap();
        assert!(!added.inputs.matches_current());
    }

    #[test]
    fn workspace_globs_track_new_members_and_missing_member_manifests() {
        let f = Fixture::new();
        f.write(
            "Cargo.toml",
            "[workspace]\nmembers=['crates/*']\nresolver='2'\n",
        );
        f.write("crates/one/Cargo.toml", &MANIFEST.replace("tracked", "one"));
        f.write("crates/one/src/lib.rs", "pub fn one() {}\n");
        std::fs::create_dir_all(f.0.join("crates/two")).unwrap();
        let first = f.load();
        assert_eq!(first.members.len(), 1);
        assert!(first.inputs.matches_current());
        f.write("crates/two/Cargo.toml", &MANIFEST.replace("tracked", "two"));
        f.write("crates/two/src/lib.rs", "pub fn two() {}\n");
        assert!(!first.inputs.matches_current());
        let second = f.load();
        assert_eq!(second.members.len(), 2);
        assert!(second.inputs.matches_current());
        f.write(
            "crates/three/Cargo.toml",
            &MANIFEST.replace("tracked", "three"),
        );
        f.write("crates/three/src/lib.rs", "pub fn three() {}\n");
        assert!(!second.inputs.matches_current());
    }

    #[test]
    fn nearest_manifest_negative_search_and_symlink_identity_are_recorded() {
        let f = Fixture::new();
        f.package();
        let nested = f.0.join("nested/deep");
        std::fs::create_dir_all(&nested).unwrap();
        let workspace = manifest::load_from_directory(&nested).unwrap();
        assert!(workspace.inputs.matches_current());
        f.write("nested/Cargo.toml", MANIFEST);
        assert!(!workspace.inputs.matches_current());
        let link = f.0.join("manifest-link");
        std::os::unix::fs::symlink("Cargo.toml", &link).unwrap();
        let linked = manifest::load(&link).unwrap();
        assert!(linked.inputs.matches_current());
        f.write("other.toml", MANIFEST);
        std::fs::remove_file(&link).unwrap();
        std::os::unix::fs::symlink("other.toml", &link).unwrap();
        assert!(!linked.inputs.matches_current());
    }

    #[test]
    fn discovery_errors_exhaustion_and_uncontrolled_tools_cannot_issue_identity() {
        let f = Fixture::new();
        f.package();
        let mut recorder = Recorder::new();
        recorder.remaining = 0;
        assert!(recorder.is_file(&f.0.join("Cargo.toml")));
        let exhausted = recorder.finish();
        assert!(exhausted.identity().is_none());
        assert!(!exhausted.matches_current());
        let mut recorder = Recorder::new();
        recorder.uncontrolled();
        let uncontrolled = recorder.finish();
        assert!(uncontrolled.identity().is_none());
        assert!(!uncontrolled.matches_current());
        let mut recorder = Recorder::new();
        assert!(recorder.read_to_string(&f.0).is_err());
        assert!(recorder.finish().identity().is_none());
        let mut recorder = Recorder::new();
        let value = recorder.read_dir(&f.0.join("absent"));
        assert_eq!(value.unwrap_err().kind(), io::ErrorKind::NotFound);
        let absent = recorder.finish();
        assert!(absent.matches_current());
        std::fs::create_dir(f.0.join("absent")).unwrap();
        assert!(!absent.matches_current());
    }

    #[test]
    fn configuration_child_probe() {
        let Some(expected) = std::env::var_os("JR_CONFIG_CHILD") else {
            return;
        };
        let workspace = manifest::load(Path::new("Cargo.toml")).unwrap();
        assert_eq!(
            workspace.inputs.identity().is_some(),
            expected == "eligible"
        );
        assert_eq!(workspace.inputs.matches_current(), expected == "eligible");
    }

    #[test]
    fn production_configuration_observes_ancestors_and_cargo_home() {
        let f = Fixture::new();
        f.write("app/Cargo.toml", MANIFEST);
        f.write("app/src/main.rs", "fn main() {}\n");
        std::fs::create_dir(f.0.join("home")).unwrap();
        let check = |eligible| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "project::inputs::tests::configuration_child_probe",
                    "--nocapture",
                ])
                .current_dir(f.0.join("app"))
                .env("CARGO_HOME", "../home")
                .env(
                    "JR_CONFIG_CHILD",
                    if eligible { "eligible" } else { "ineligible" },
                )
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        };
        check(true);
        for path in [
            ".cargo/config",
            ".cargo/config.toml",
            "home/config",
            "home/config.toml",
        ] {
            f.write(path, "[build]\njobs=2\n");
            check(false);
            std::fs::remove_file(f.0.join(path)).unwrap();
            check(true);
        }
    }

    #[test]
    fn environment_child_probe() {
        if std::env::var_os("JR_DISCOVERY_CHILD").is_none() {
            return;
        }
        let mut recorder = Recorder::new();
        let key = std::env::var("JR_DISCOVERY_KEY").unwrap();
        let actual = recorder.env(&key);
        let receipt = recorder.finish();
        assert_eq!(receipt.environment(&key), actual.as_deref());
        assert!(receipt.matches_current());
        let mut exhausted = Recorder::new();
        exhausted.remaining = 0;
        let frozen = exhausted.env(&key);
        let exhausted = exhausted.finish();
        assert_eq!(exhausted.environment(&key), frozen.as_deref());
        assert!(exhausted.identity().is_none());
        println!("discovery-identity:{:?}", receipt.identity().unwrap());
        let mut forged = (*receipt).clone();
        forged.observations[0].value =
            Observed::Environment(Some("a deliberately different environment value".into()));
        assert!(!forged.matches_current());
    }

    #[test]
    fn environment_absence_empty_and_values_have_distinct_identities() {
        for key in ["CARGO_TARGET_DIR", "CARGO_HOME", "HOME"] {
            let mut identities = std::collections::BTreeSet::new();
            for value in [None, Some(""), Some("one"), Some("two")] {
                let mut command = std::process::Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "project::inputs::tests::environment_child_probe",
                        "--nocapture",
                        "--test-threads=1",
                    ])
                    .env("JR_DISCOVERY_CHILD", "1")
                    .env("JR_DISCOVERY_KEY", key)
                    .env_remove(key);
                if let Some(value) = value {
                    command.env(key, value);
                }
                let output = command.output().unwrap();
                assert!(output.status.success(), "{output:?}");
                let stdout = String::from_utf8(output.stdout).unwrap();
                let identity = stdout
                    .lines()
                    .find_map(|line| {
                        line.split_once("discovery-identity:")
                            .map(|(_, identity)| identity)
                    })
                    .unwrap();
                assert!(identities.insert(identity.to_owned()));
            }
        }
    }

    #[test]
    fn path_binding_identity_is_bounded_and_conflicts_fail_closed() {
        let f = Fixture::new();
        let path = f.0.join("dep");
        let mut one = Recorder::new();
        assert!(one.bind_path_package(&path, 1).is_ok());
        assert!(one.bind_path_package(&path, 1).is_ok());
        let one = one.finish();
        assert_eq!(one.path_package(&path), Some(1));
        let mut two = Recorder::new();
        assert!(two.bind_path_package(&path, 2).is_ok());
        assert_ne!(one.identity(), two.finish().identity());
        let mut conflicting = Recorder::new();
        assert!(conflicting.bind_path_package(&path, 1).is_ok());
        assert!(conflicting.bind_path_package(&path, 2).is_err());
        assert!(conflicting.finish().identity().is_none());
        let mut exhausted = Recorder::new();
        exhausted.remaining = 0;
        assert!(exhausted.bind_path_package(&path, 1).is_ok());
        let exhausted = exhausted.finish();
        assert_eq!(exhausted.path_package(&path), Some(1));
        assert!(exhausted.identity().is_none());
        assert!(!exhausted.matches_current());
        let mut bounded = Recorder::new();
        bounded.binding_remaining = 0;
        assert!(bounded.bind_path_package(&path, 1).is_err());
        let bounded = bounded.finish();
        assert!(bounded.path_package(&path).is_none());
        assert!(bounded.identity().is_none());
        assert!(!bounded.matches_current());
    }

    #[test]
    fn path_planning_uses_observed_identity_after_symlink_mutation() {
        let f = Fixture::new();
        f.write("Cargo.toml", &format!("{MANIFEST}\n[dependencies]\ndep={{path='selected', optional=true}}\n[features]\nuse_dep=['dep:dep','dep/marker']\n"));
        f.write("src/main.rs", "fn main() {}\n");
        for dir in ["one", "two"] {
            f.write(
                &format!("{dir}/Cargo.toml"),
                &format!(
                    "{}\n[features]\nmarker=[]\n",
                    MANIFEST.replace("tracked", "dep")
                ),
            );
            f.write(&format!("{dir}/src/lib.rs"), "pub fn value() {}\n");
        }
        let selected = f.0.join("selected");
        std::os::unix::fs::symlink("one", &selected).unwrap();
        let ws = f.load();
        let root = ws.members[0];
        let expected = ws
            .packages
            .iter()
            .position(|p| p.root == f.0.join("one"))
            .unwrap();
        let plan = |ws: &manifest::Workspace| {
            let features =
                manifest::resolve_features(ws, &[(root, vec!["use_dep".into()], false)], false)
                    .unwrap();
            assert!(features.enabled[expected].contains("marker"));
            let mut unresolved = Vec::new();
            let visible = manifest::visible_dependencies(
                ws,
                root,
                false,
                &features.active_deps[root],
                &mut unresolved,
            );
            assert!(unresolved.is_empty(), "{unresolved:?}");
            assert_eq!(visible, vec![("dep".into(), expected)]);
        };
        plan(&ws);
        assert!(ws.inputs.matches_current());
        std::fs::remove_file(&selected).unwrap();
        std::os::unix::fs::symlink("two", &selected).unwrap();
        assert!(!ws.inputs.matches_current());
        // A changed link must not silently redirect an already loaded plan.
        plan(&ws);
        let fresh = f.load();
        assert!(fresh.packages.iter().any(|p| p.root == f.0.join("two")));
        std::fs::remove_file(&selected).unwrap();
        assert!(!ws.inputs.matches_current());
        plan(&ws);
        // Missing paths cannot enter a freshly loaded successful workspace.
        assert!(manifest::load(&f.0.join("Cargo.toml")).is_err());
        std::os::unix::fs::symlink("one", &selected).unwrap();
        assert!(ws.inputs.matches_current());
        f.write("one/Cargo.toml", &MANIFEST.replace("tracked", "dep"));
        assert!(!ws.inputs.matches_current());
    }

    #[test]
    fn directory_order_and_non_utf8_names_are_lossless() {
        use std::os::unix::ffi::OsStringExt;
        let f = Fixture::new();
        f.write("z", "");
        f.write("a", "");
        let raw = OsString::from_vec(vec![b'x', 0xff]);
        std::fs::write(f.0.join(&raw), "").unwrap();
        let mut one = Recorder::new();
        let paths = one.read_dir(&f.0).unwrap();
        assert_eq!(paths.len(), 3);
        assert!(paths.contains(&f.0.join(&raw)));
        let one = one.finish();
        let mut two = Recorder::new();
        two.read_dir(&f.0).unwrap();
        let two = two.finish();
        assert_eq!(one.identity(), two.identity());
        assert!(one.matches_current());
        std::fs::remove_file(f.0.join(&raw)).unwrap();
        assert!(!one.matches_current());
    }

    #[test]
    fn registry_bindings_are_bounded_and_never_authorize_sharing() {
        let key = manifest::LockedRegistryPackage {
            name: "dep".into(),
            version: "0.1.0".into(),
            source: "registry+https://github.com/rust-lang/crates.io-index".into(),
            checksum: "0".repeat(64),
        };
        let binding = RegistryBinding {
            index: 1,
            root: "/source".into(),
            archive: "/archive".into(),
        };
        let mut recorder = Recorder::new();
        recorder
            .bind_registry_package(&key, binding.clone())
            .unwrap();
        recorder
            .bind_registry_package(&key, binding.clone())
            .unwrap();
        let mut changed = binding.clone();
        changed.index = 2;
        assert!(recorder.bind_registry_package(&key, changed).is_err());
        let receipt = recorder.finish();
        assert_eq!(receipt.registry_package(&key), Some(&binding));
        assert!(receipt.identity().is_none());
        assert!(!receipt.matches_current());
        let mut exhausted = Recorder::new();
        exhausted.binding_remaining = 0;
        assert!(exhausted.bind_registry_package(&key, binding).is_err());
        assert!(exhausted.finish().registry_package(&key).is_none());
    }

    #[test]
    fn registry_planning_freezes_selection_but_reauthenticates_bytes() {
        let f = Fixture::new();
        f.write(
            "one/registry/src/index.crates.io-1949cf8c6b5b557f/dep-0.1.0/Cargo.toml",
            "[package]\nname='dep'\nversion='0.1.0'\nedition='2024'\n[features]\nmarker=[]\n",
        );
        f.write(
            "one/registry/src/index.crates.io-1949cf8c6b5b557f/dep-0.1.0/src/lib.rs",
            "pub fn value() -> i32 { 7 }\n",
        );
        let src =
            f.0.join("one/registry/src/index.crates.io-1949cf8c6b5b557f");
        let cache =
            f.0.join("one/registry/cache/index.crates.io-1949cf8c6b5b557f");
        std::fs::create_dir_all(&cache).unwrap();
        let archive = cache.join("dep-0.1.0.crate");
        assert!(
            std::process::Command::new("tar")
                .arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(&src)
                .arg("dep-0.1.0")
                .status()
                .unwrap()
                .success()
        );
        let sum = std::process::Command::new("sha256sum")
            .arg(&archive)
            .output()
            .unwrap();
        assert!(sum.status.success());
        let checksum = String::from_utf8(sum.stdout).unwrap();
        let checksum = checksum.split_whitespace().next().unwrap();
        f.write("Cargo.toml", &format!("{MANIFEST}[dependencies]\ndep={{version='=0.1.0',optional=true}}\n[features]\nuse_dep=['dep:dep','dep/marker']\n"));
        f.write("Cargo.lock", &format!("version=4\n[[package]]\nname='tracked'\nversion='0.1.0'\ndependencies=['dep']\n[[package]]\nname='dep'\nversion='0.1.0'\nsource='registry+https://github.com/rust-lang/crates.io-index'\nchecksum='{checksum}'\n"));
        f.write("src/main.rs", "fn main() { assert!(dep::value() == 7); }\n");
        std::os::unix::fs::symlink("one", f.0.join("home")).unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "project::inputs::tests::registry_planning_child",
                "--nocapture",
            ])
            .env("JR_REGISTRY_PLANNING_CHILD", &f.0)
            .env("CARGO_HOME", f.0.join("home"))
            .env("CARGO_TARGET_DIR", f.0.join("target"))
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
    }

    #[test]
    fn registry_planning_child() {
        let Some(root) = std::env::var_os("JR_REGISTRY_PLANNING_CHILD") else {
            return;
        };
        let root = PathBuf::from(root);
        let path = root.join("Cargo.toml");
        let ws = manifest::load(&path).unwrap();
        let index = ws.packages.iter().position(|p| p.registry).unwrap();
        let key = match &ws.packages[ws.members[0]].dependencies[0].source {
            manifest::DependencySource::Registry(key) => key.clone(),
            _ => panic!("not registry"),
        };
        let binding = ws.inputs.registry_package(&key).unwrap().clone();
        assert_eq!(binding.index, index);
        assert!(ws.inputs.identity().is_none());
        let plan = |ws: &manifest::Workspace, available: bool| {
            let resolved = manifest::resolve_features(
                ws,
                &[(ws.members[0], vec!["use_dep".into()], false)],
                false,
            )
            .unwrap();
            assert!(resolved.enabled[index].contains("marker"));
            let mut unresolved = Vec::new();
            let visible = manifest::visible_dependencies(
                ws,
                ws.members[0],
                false,
                &resolved.active_deps[ws.members[0]],
                &mut unresolved,
            );
            if available {
                assert_eq!(visible, vec![("dep".into(), index)]);
                assert!(unresolved.is_empty());
            } else {
                assert!(visible.is_empty());
                assert_eq!(unresolved.len(), 1);
            }
        };
        plan(&ws, true);
        for workers in [1, 8] {
            let driver = crate::project::driver::Driver::new(
                &path,
                crate::project::driver::Selection {
                    workers,
                    features: vec!["use_dep".into()],
                    ..Default::default()
                },
            )
            .unwrap();
            for command in [
                crate::project::driver::Command::Check,
                crate::project::driver::Command::Build,
                crate::project::driver::Command::Run,
            ] {
                assert_eq!(driver.run(command).unwrap(), 0);
            }
        }
        // A new cache becomes visible through the same CARGO_HOME spelling.
        // Existing planning must neither select it nor lose its feature binding.
        let other_src = root.join("two/registry/src/index.crates.io-1949cf8c6b5b557f");
        let other_cache = root.join("two/registry/cache/index.crates.io-1949cf8c6b5b557f");
        std::fs::create_dir_all(&other_src).unwrap();
        std::fs::create_dir_all(&other_cache).unwrap();
        std::fs::copy(&binding.archive, other_cache.join("dep-0.1.0.crate")).unwrap();
        assert!(
            std::process::Command::new("tar")
                .arg("-xzf")
                .arg(&binding.archive)
                .arg("-C")
                .arg(&other_src)
                .status()
                .unwrap()
                .success()
        );
        std::fs::remove_file(root.join("home")).unwrap();
        std::os::unix::fs::symlink("two", root.join("home")).unwrap();
        plan(&ws, true);
        let fresh = manifest::load(&path).unwrap();
        assert_ne!(
            fresh.inputs.registry_package(&key).unwrap().root,
            binding.root
        );
        let original = std::fs::read(&binding.archive).unwrap();
        std::fs::write(&binding.archive, b"changed").unwrap();
        plan(&ws, false);
        plan(&fresh, true);
        std::fs::write(&binding.archive, original).unwrap();
        let source = binding.root.join("src/lib.rs");
        let original = std::fs::read(&source).unwrap();
        std::fs::write(&source, b"pub fn value() -> i32 { 8 }\n").unwrap();
        plan(&ws, false);
        std::fs::write(&source, &original).unwrap();
        plan(&ws, true);
        std::fs::remove_file(&source).unwrap();
        plan(&ws, false);
        std::fs::write(&source, original).unwrap();
        // Missing optional discovery cannot acquire a binding after the fact.
        std::fs::rename(other_src.join("dep-0.1.0"), other_src.join("saved")).unwrap();
        let absent = manifest::load(&path).unwrap();
        assert!(absent.inputs.registry_package(&key).is_none());
        std::fs::rename(other_src.join("saved"), other_src.join("dep-0.1.0")).unwrap();
        let mut unresolved = Vec::new();
        assert!(
            manifest::visible_dependencies(
                &absent,
                absent.members[0],
                false,
                &["dep".into()].into(),
                &mut unresolved
            )
            .is_empty()
        );
        assert_eq!(unresolved.len(), 1);
    }
}
