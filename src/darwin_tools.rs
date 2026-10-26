//! Host-local, explicitly provisioned Apple SDKs. Never downloads Apple material.
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

const SCHEMA: u32 = 1;
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct Manifest {
    schema: u32,
    files: BTreeMap<String, String>,
}

pub fn supported(target: &str) -> bool {
    matches!(target, "x86_64-apple-darwin" | "aarch64-apple-darwin")
}

fn store() -> Result<PathBuf> {
    Ok(crate::toolchain::root()?.join("apple-sdks/v1"))
}

fn valid_id(id: &str) -> bool {
    id.len() == 64
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

// Materialize internal SDK symlinks so neither subsequent source changes nor
// runtime symlink traversal can redirect the managed SDK outside its inventory.
fn copy_tree(
    source: &Path,
    destination: &Path,
    boundary: &Path,
    ancestors: &mut Vec<PathBuf>,
) -> Result<()> {
    let canonical = source.canonicalize()?;
    if !canonical.starts_with(boundary) {
        bail!("Apple SDK symlink escapes source SDK: {}", source.display());
    }
    let metadata = std::fs::metadata(&canonical)?;
    if metadata.is_dir() {
        if ancestors.contains(&canonical) {
            bail!("Apple SDK directory symlink cycle");
        }
        ancestors.push(canonical.clone());
        std::fs::create_dir(destination)?;
        for entry in std::fs::read_dir(&canonical)? {
            let entry = entry?;
            copy_tree(
                &entry.path(),
                &destination.join(entry.file_name()),
                boundary,
                ancestors,
            )?;
        }
        ancestors.pop();
    } else if metadata.is_file() {
        std::fs::copy(&canonical, destination)?;
        let mut permissions = std::fs::metadata(destination)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(destination, permissions)?;
    } else {
        bail!("Apple SDK contains a non-regular file");
    }
    Ok(())
}

fn inventory(root: &Path, dir: &Path, files: &mut BTreeMap<String, String>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            inventory(root, &path, files)?;
        } else if kind.is_file() {
            let relative = path
                .strip_prefix(root)?
                .to_str()
                .context("non-UTF8 Apple SDK filename")?
                .to_owned();
            files.insert(relative, crate::toolchain::sha256(&path)?);
        } else {
            bail!("managed Apple SDK contains a symlink or special file");
        }
    }
    Ok(())
}

fn minimum_sdk(sdk: &Path) -> Result<()> {
    for name in [
        "usr/include/stdio.h",
        "usr/include/stdlib.h",
        "usr/include/TargetConditionals.h",
        "usr/lib/libSystem.tbd",
        "System/Library/Frameworks/Foundation.framework/Foundation.tbd",
        "System/Library/Frameworks/CoreFoundation.framework/CoreFoundation.tbd",
        "System/Library/Frameworks/AppKit.framework/AppKit.tbd",
    ] {
        if !sdk.join(name).is_file() {
            bail!("Apple SDK is missing required header/framework stub: {name}");
        }
    }
    if !sdk.join("SDKSettings.json").is_file() && !sdk.join("SDKSettings.plist").is_file() {
        bail!("Apple SDK has no SDKSettings metadata");
    }
    for name in [
        "usr/lib/libSystem.tbd",
        "System/Library/Frameworks/Foundation.framework/Foundation.tbd",
        "System/Library/Frameworks/CoreFoundation.framework/CoreFoundation.tbd",
        "System/Library/Frameworks/AppKit.framework/AppKit.tbd",
    ] {
        let stub = std::fs::read_to_string(sdk.join(name))?;
        let tokens: Vec<_> = stub
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .collect();
        if !tokens.contains(&"x86_64")
            || !tokens.contains(&"arm64")
            || !(tokens.contains(&"macos") || tokens.contains(&"macosx"))
        {
            bail!("Apple SDK stub must support macOS on both x86_64 and arm64: {name}");
        }
    }
    Ok(())
}

/// Administrative operation on the hosted account itself. The caller must have
/// independently obtained and be authorized to use this SDK. Never call with a
/// build request's local path, and never transport SDKs as build inputs.
pub fn provision(source: &Path) -> Result<String> {
    provision_into(source, &store()?)
}

fn provision_into(source: &Path, root: &Path) -> Result<String> {
    let boundary = source
        .canonicalize()
        .context("opening user-provided Apple SDK")?;
    std::fs::create_dir_all(root)?;
    if root.canonicalize()?.starts_with(&boundary) {
        bail!("Apple SDK store must not be inside the source SDK");
    }
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos();
    let stage = root.join(format!(".staging-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&stage)?;
    let result = (|| {
        let sdk = stage.join("sdk");
        copy_tree(&boundary, &sdk, &boundary, &mut Vec::new())?;
        minimum_sdk(&sdk)?;
        let mut files = BTreeMap::new();
        inventory(&sdk, &sdk, &mut files)?;
        let manifest = stage.join("manifest.json");
        std::fs::write(
            &manifest,
            serde_json::to_vec(&Manifest {
                schema: SCHEMA,
                files,
            })?,
        )?;
        let id = crate::toolchain::sha256(&manifest)?;
        let mut permissions = std::fs::metadata(&manifest)?.permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&manifest, permissions)?;
        let destination = root.join(&id);
        if destination.exists() {
            validate(root, &id)?;
        } else if let Err(error) = std::fs::rename(&stage, &destination) {
            if destination.exists() {
                validate(root, &id)?;
            } else {
                return Err(error.into());
            }
        }
        Ok(id)
    })();
    if stage.exists() {
        let _ = std::fs::remove_dir_all(&stage);
    }
    result
}

fn validate(root: &Path, id: &str) -> Result<PathBuf> {
    if !valid_id(id) {
        bail!("managed Apple SDK ID must be a lowercase SHA-256 digest");
    }
    let directory = root.join(id);
    for path in [
        &directory,
        &directory.join("manifest.json"),
        &directory.join("sdk"),
    ] {
        if std::fs::symlink_metadata(path)?.file_type().is_symlink() {
            bail!("managed Apple SDK must not be redirected by symlinks");
        }
    }
    let manifest_path = directory.join("manifest.json");
    if crate::toolchain::sha256(&manifest_path)? != id {
        bail!("Apple SDK manifest integrity mismatch");
    }
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(manifest_path)?)?;
    if manifest.schema != SCHEMA {
        bail!("unsupported Apple SDK manifest version");
    }
    let sdk = directory.join("sdk");
    let mut files = BTreeMap::new();
    inventory(&sdk, &sdk, &mut files)?;
    if files != manifest.files {
        bail!("Apple SDK content integrity mismatch");
    }
    minimum_sdk(&sdk)?;
    Ok(sdk)
}

/// Configure only the owned target build command, never the agent process or
/// user's shell. Caller selects cargo-zigbuild and passes the matching --target.
pub fn apply(cmd: &mut Command, target: &str, sdk_id: &str) -> Result<()> {
    if !supported(target) {
        bail!("unsupported managed Darwin target: {target}");
    }
    let sdk = validate(&store()?, sdk_id)?;
    crate::release_tools::apply(cmd, crate::release_tools::ensure()?)?;
    cmd.env("SDKROOT", sdk);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> PathBuf {
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("justrust-darwin-{}-{nonce}", std::process::id()));
        for name in [
            "usr/include/stdio.h",
            "usr/include/stdlib.h",
            "usr/include/TargetConditionals.h",
            "SDKSettings.json",
            "usr/lib/libSystem.tbd",
            "System/Library/Frameworks/Foundation.framework/Foundation.tbd",
            "System/Library/Frameworks/CoreFoundation.framework/CoreFoundation.tbd",
            "System/Library/Frameworks/AppKit.framework/AppKit.tbd",
        ] {
            let path = root.join("source").join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "targets: [ x86_64-macos, arm64-macos ]").unwrap();
        }
        root
    }
    #[test]
    fn targets_and_ids_are_strict() {
        assert!(supported("aarch64-apple-darwin"));
        assert!(supported("x86_64-apple-darwin"));
        assert!(!supported("aarch64-apple-ios"));
        assert!(!valid_id("../../sdk"));
        assert!(!valid_id(&"A".repeat(64)));
    }
    #[test]
    fn immutable_inventory_detects_additions_and_missing_files() {
        let root = fixture();
        let store = root.join("store");
        let id = provision_into(&root.join("source"), &store).unwrap();
        assert_eq!(id, provision_into(&root.join("source"), &store).unwrap());
        let sdk = validate(&store, &id).unwrap();
        std::fs::write(sdk.join("extra"), "changed").unwrap();
        assert!(validate(&store, &id).is_err());
        std::fs::remove_file(sdk.join("extra")).unwrap();
        std::fs::remove_file(sdk.join("usr/include/stdio.h")).unwrap();
        assert!(validate(&store, &id).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn manifest_and_content_tampering_are_rejected() {
        let root = fixture();
        let store = root.join("store");
        let id = provision_into(&root.join("source"), &store).unwrap();
        let sdk = validate(&store, &id).unwrap();
        // Rename replacement avoids relying on platform-specific readonly bits.
        let replacement = root.join("replacement");
        std::fs::write(&replacement, "corruption").unwrap();
        std::fs::rename(&replacement, sdk.join("usr/include/stdio.h")).unwrap();
        assert!(validate(&store, &id).is_err());
        std::fs::write(&replacement, b"{}").unwrap();
        std::fs::rename(&replacement, store.join(&id).join("manifest.json")).unwrap();
        assert!(validate(&store, &id).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(unix)]
    fn internal_symlinks_are_materialized_and_cycles_rejected() {
        let root = fixture();
        std::os::unix::fs::symlink("usr/include/stdio.h", root.join("source/alias")).unwrap();
        let store = root.join("store");
        let id = provision_into(&root.join("source"), &store).unwrap();
        let sdk = validate(&store, &id).unwrap();
        assert!(
            std::fs::symlink_metadata(sdk.join("alias"))
                .unwrap()
                .is_file()
        );
        std::fs::remove_file(sdk.join("alias")).unwrap();
        std::os::unix::fs::symlink("usr/include/stdio.h", sdk.join("alias")).unwrap();
        assert!(validate(&store, &id).is_err());
        std::os::unix::fs::symlink(".", root.join("source/cycle")).unwrap();
        assert!(provision_into(&root.join("source"), &store).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn requires_both_architectures() {
        let root = fixture();
        std::fs::write(root.join("source/usr/lib/libSystem.tbd"), "x86_64").unwrap();
        assert!(provision_into(&root.join("source"), &root.join("store")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[cfg(unix)]
    fn rejects_escaping_symlinks() {
        let root = fixture();
        std::os::unix::fs::symlink("/etc/passwd", root.join("source/escape")).unwrap();
        assert!(provision_into(&root.join("source"), &root.join("store")).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
