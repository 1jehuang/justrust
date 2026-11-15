//! Host-authorized, build-only public toolchains. No user environment supplies
//! authority or a path. Installing/mutating toolchains never consults this module.
//!
//! `/opt/justrust-public-toolchains/public.json` opts the host in:
//! `{"version":1,"enabled":true,"toolchains":[{"id":"jr-abcdef0","target":"x86_64-unknown-linux-gnu"}]}`
//! Each listed complete bundle lives at `<root>/<id>` with the usual Installed
//! marker. The manifest, every ancestor, and the entire bundle must be root
//! owned, non-group/world-writable and symlink-free. Publish fully assembled
//! immutable bundles, never modify them while clients may be using them.

use super::{Installed, MARKER, Spec, TARGET, Toolchain, installed_in, is_executable};
use serde::Deserialize;
use std::path::Path;

const ROOT: &str = "/opt/justrust-public-toolchains";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    enabled: bool,
    toolchains: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    id: String,
    target: String,
}

/// Only called for an explicit resolved pin, never for the unpinned default.
/// Private installations are checked first, including the private staged area.
pub(super) fn for_build(spec: &Spec) -> Option<Toolchain> {
    select(
        spec,
        &super::root().ok()?,
        Path::new(ROOT),
        TARGET,
        0,
        Path::new("/"),
    )
}

fn select(
    spec: &Spec,
    private: &Path,
    public: &Path,
    target: &str,
    owner: u32,
    trust_boundary: &Path,
) -> Option<Toolchain> {
    if let Ok(Some(tc)) = installed_in(spec, private) {
        return Some(tc);
    }
    lookup(spec, public, target, owner, trust_boundary)
}

#[cfg(unix)]
fn trusted(path: &Path, owner: u32) -> bool {
    use std::os::unix::fs::MetadataExt;
    std::fs::symlink_metadata(path).is_ok_and(|m| {
        !m.file_type().is_symlink()
            && (m.is_file() || m.is_dir())
            && m.uid() == owner
            && m.mode() & 0o022 == 0
    })
}

#[cfg(not(unix))]
fn trusted(_: &Path, _: u32) -> bool {
    false
}

fn trusted_tree(path: &Path, owner: u32) -> Option<()> {
    if !trusted(path, owner) {
        return None;
    }
    if path.is_dir() {
        for entry in std::fs::read_dir(path).ok()? {
            trusted_tree(&entry.ok()?.path(), owner)?;
        }
    }
    Some(())
}

fn lookup(
    spec: &Spec,
    root: &Path,
    target: &str,
    owner: u32,
    boundary: &Path,
) -> Option<Toolchain> {
    if !spec.is_jr() || Spec::parse(&spec.id()).ok().as_ref() != Some(spec) {
        return None;
    }
    // Production boundary is /, including /opt and the filesystem root. Tests
    // inject a temporary trust boundary and its owner, never process-global env.
    if !root.starts_with(boundary) {
        return None;
    }
    for ancestor in root.ancestors() {
        if !trusted(ancestor, owner) || !ancestor.is_dir() {
            return None;
        }
        if ancestor == boundary {
            break;
        }
    }
    let manifest = root.join("public.json");
    if !trusted(&manifest, owner) || !manifest.is_file() {
        return None;
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(manifest).ok()?).ok()?;
    if manifest.version != 1
        || !manifest.enabled
        || !manifest
            .toolchains
            .iter()
            .any(|e| e.id == spec.id() && e.target == target)
    {
        return None;
    }
    let dir = root.join(spec.id());
    trusted_tree(&dir, owner)?;
    let info: Installed = serde_json::from_slice(&std::fs::read(dir.join(MARKER)).ok()?).ok()?;
    if info.id != spec.id()
        || !["rustc", "cargo"]
            .iter()
            .all(|name| is_executable(&dir.join("bin").join(crate::sys::exe_name(name))))
    {
        return None;
    }
    Some(Toolchain {
        id: spec.id(),
        dir,
        nightly: true,
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    struct TempDir(std::path::PathBuf);

    impl TempDir {
        fn new() -> Self {
            use std::os::unix::fs::DirBuilderExt;
            use std::sync::atomic::{AtomicU64, Ordering};
            static NEXT: AtomicU64 = AtomicU64::new(0);
            loop {
                let path = std::env::temp_dir().join(format!(
                    "jr-public-{}-{}",
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed)
                ));
                match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                    Ok(()) => return Self(path),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(e) => panic!("creating test directory: {e}"),
                }
            }
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    struct Fixture {
        temp: TempDir,
        root: std::path::PathBuf,
        private: std::path::PathBuf,
        spec: Spec,
        uid: u32,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new();
            let root = temp.path().join("public");
            let private = temp.path().join("private");
            std::fs::create_dir(&root).unwrap();
            let spec = Spec::parse("jr-abcdef0").unwrap();
            let uid = std::fs::metadata(temp.path()).unwrap().uid();
            let f = Self {
                temp,
                root,
                private,
                spec,
                uid,
            };
            f.manifest(true, TARGET, &f.spec.id());
            f.bundle(&f.root);
            f
        }
        fn manifest(&self, enabled: bool, target: &str, id: &str) {
            std::fs::write(
                self.root.join("public.json"),
                serde_json::to_vec(&serde_json::json!({
                    "version": 1, "enabled": enabled, "toolchains": [{"id":id,"target":target}]
                }))
                .unwrap(),
            )
            .unwrap();
        }
        fn bundle(&self, root: &Path) {
            let dir = root.join(self.spec.id());
            std::fs::create_dir_all(dir.join("bin")).unwrap();
            let marker = Installed {
                id: self.spec.id(),
                ..Installed::default()
            };
            std::fs::write(dir.join(MARKER), serde_json::to_vec(&marker).unwrap()).unwrap();
            for bin in ["rustc", "cargo"] {
                let path = dir.join("bin").join(bin);
                std::fs::write(&path, "executable fixture").unwrap();
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        }
        fn get(&self) -> Option<Toolchain> {
            select(
                &self.spec,
                &self.private,
                &self.root,
                TARGET,
                self.uid,
                self.temp.path(),
            )
        }
    }

    #[test]
    fn complete_public_bundle_and_private_precedence() {
        let f = Fixture::new();
        assert_eq!(f.get().unwrap().dir, f.root.join(f.spec.id()));
        f.bundle(&f.private);
        assert_eq!(f.get().unwrap().dir, f.private.join(f.spec.id()));
        // Mutable lookup remains private-only.
        assert!(installed_in(&f.spec, &f.private).unwrap().is_some());
        std::fs::remove_dir_all(&f.private).unwrap();
        assert!(installed_in(&f.spec, &f.private).unwrap().is_none());
    }

    #[test]
    fn rejects_untrusted_or_incomplete_public_bundles() {
        for case in [
            "disabled",
            "target",
            "id",
            "owner",
            "marker",
            "missing",
            "executable",
            "manifest-link",
            "binary-link",
            "root-link",
            "ancestor-write",
            "manifest-write",
            "library-write",
        ] {
            let f = Fixture::new();
            let dir = f.root.join(f.spec.id());
            match case {
                "disabled" => f.manifest(false, TARGET, &f.spec.id()),
                "target" => f.manifest(true, "wrong-target", &f.spec.id()),
                "id" => f.manifest(true, TARGET, "jr-1234567"),
                "owner" => {
                    assert!(
                        lookup(
                            &f.spec,
                            &f.root,
                            TARGET,
                            f.uid.wrapping_add(1),
                            f.temp.path()
                        )
                        .is_none()
                    );
                    continue;
                }
                "marker" => std::fs::write(dir.join(MARKER), b"{}").unwrap(),
                "missing" => std::fs::remove_file(dir.join("bin/cargo")).unwrap(),
                "executable" => std::fs::set_permissions(
                    dir.join("bin/rustc"),
                    std::fs::Permissions::from_mode(0o644),
                )
                .unwrap(),
                "manifest-link" | "binary-link" => {
                    let path = if case == "manifest-link" {
                        f.root.join("public.json")
                    } else {
                        dir.join("bin/rustc")
                    };
                    let saved = f.temp.path().join("saved");
                    std::fs::rename(&path, &saved).unwrap();
                    symlink(saved, path).unwrap();
                }
                "root-link" => {
                    let saved = f.temp.path().join("saved");
                    std::fs::rename(&f.root, &saved).unwrap();
                    symlink(saved, &f.root).unwrap();
                }
                "ancestor-write" => {
                    std::fs::set_permissions(f.temp.path(), std::fs::Permissions::from_mode(0o777))
                        .unwrap()
                }
                "manifest-write" => std::fs::set_permissions(
                    f.root.join("public.json"),
                    std::fs::Permissions::from_mode(0o666),
                )
                .unwrap(),
                "library-write" => {
                    std::fs::write(dir.join("lib.so"), "library").unwrap();
                    std::fs::set_permissions(
                        dir.join("lib.so"),
                        std::fs::Permissions::from_mode(0o666),
                    )
                    .unwrap();
                }
                _ => unreachable!(),
            }
            assert!(f.get().is_none(), "accepted {case}");
        }
    }

    #[test]
    fn official_and_missing_manifest_never_opt_in() {
        let f = Fixture::new();
        assert!(
            lookup(
                &Spec::parse("1.90.0").unwrap(),
                &f.root,
                TARGET,
                f.uid,
                f.temp.path()
            )
            .is_none()
        );
        std::fs::remove_file(f.root.join("public.json")).unwrap();
        assert!(f.get().is_none());
    }
}
