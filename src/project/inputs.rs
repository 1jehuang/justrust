//! Bounded observations from native project discovery. No cache authorization.
//! Each loader owns its recorder. Only the immutable receipt crosses into the
//! source graph; no mutable recorder or filesystem handle is published.
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
/// tools, subsequent planning IO and atomic snapshots remain outside this scope.
#[derive(Clone, Debug)]
pub struct DiscoveryInputs {
    environment: std::collections::BTreeMap<String, Option<OsString>>,
    observations: Vec<Observation>,
    complete: bool,
    uncontrolled: bool,
    identity: [u8; 32],
}
impl crate::engine::Fact for DiscoveryInputs {}
impl DiscoveryInputs {
    pub(crate) fn untracked() -> Arc<Self> {
        Arc::new(Self {
            environment: std::collections::BTreeMap::new(),
            observations: Vec::new(),
            complete: false,
            uncontrolled: true,
            identity: [0; 32],
        })
    }
    pub fn observation_count(&self) -> usize {
        self.observations.len()
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
    environment: std::collections::BTreeMap<String, Option<OsString>>,
    observations: Vec<Observation>,
    cwd: Option<PathBuf>,
    complete: bool,
    uncontrolled: bool,
    remaining: usize,
    identity: blake3::Hasher,
}
impl Recorder {
    pub(crate) fn new() -> Self {
        let cwd = std::env::current_dir().ok();
        let mut identity = blake3::Hasher::new();
        identity.update(b"justrust-project-discovery-v1");
        Self {
            environment: std::collections::BTreeMap::new(),
            observations: Vec::new(),
            complete: cwd.is_some(),
            cwd,
            uncontrolled: false,
            remaining: MAX_BYTES,
            identity,
        }
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
        if self.observations.len() >= MAX_ENTRIES || encoded.len() > self.remaining {
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
        let r = path.canonicalize();
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
        let r = std::fs::metadata(path).map(|m| m.is_file());
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
        let r = read_bounded(path);
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
        let r = list_bounded(path);
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
        let value = std::env::var_os(name);
        self.environment.insert(name.to_owned(), value.clone());
        self.record(Path::new(name), Observed::Environment(value.clone()));
        value
    }
    pub(crate) fn uncontrolled(&mut self) {
        self.uncontrolled = true;
    }
    pub(crate) fn finish(self) -> Arc<DiscoveryInputs> {
        Arc::new(DiscoveryInputs {
            environment: self.environment,
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
}
