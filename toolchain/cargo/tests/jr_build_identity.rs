#[allow(dead_code)]
#[path = "../build.rs"]
mod build_script;

use std::fs;
use tempfile::TempDir;

fn fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    for path in [
        "src",
        "crates",
        "credential",
        ".cargo",
        "etc/man",
        "doc/man/generated_txt",
    ] {
        fs::create_dir_all(dir.path().join(path)).unwrap();
    }
    for path in [
        "Cargo.toml",
        "Cargo.lock",
        "build.rs",
        "windows.manifest.xml",
        "JUSTRUST_BASE",
        ".cargo/config.toml",
        "src/lib.rs",
    ] {
        fs::write(dir.path().join(path), "initial").unwrap();
    }
    dir
}

#[test]
fn dirty_source_edits_change_identity_without_git_metadata() {
    let dir = fixture();
    let original = build_script::source_fingerprint(dir.path());
    assert_eq!(original.len(), 40);
    assert!(original.bytes().all(|byte| byte.is_ascii_hexdigit()));
    fs::write(dir.path().join("src/lib.rs"), "changed").unwrap();
    assert_ne!(original, build_script::source_fingerprint(dir.path()));
    fs::write(dir.path().join("src/lib.rs"), "initial").unwrap();
    assert_eq!(original, build_script::source_fingerprint(dir.path()));
}

#[test]
fn generated_targets_and_git_metadata_do_not_change_identity() {
    let dir = fixture();
    let original = build_script::source_fingerprint(dir.path());
    fs::create_dir_all(dir.path().join("crates/example/target")).unwrap();
    let with_crate = build_script::source_fingerprint(dir.path());
    assert_ne!(original, with_crate);
    fs::write(dir.path().join("crates/example/target/artifact"), "output").unwrap();
    assert_eq!(with_crate, build_script::source_fingerprint(dir.path()));
    fs::create_dir_all(dir.path().join(".git")).unwrap();
    fs::write(dir.path().join(".git/HEAD"), "unrelated commit").unwrap();
    assert_eq!(with_crate, build_script::source_fingerprint(dir.path()));
}

#[test]
fn new_sources_and_workspace_manifest_edits_change_identity() {
    let dir = fixture();
    let original = build_script::source_fingerprint(dir.path());
    fs::write(dir.path().join("src/new.rs"), "new source").unwrap();
    assert_ne!(original, build_script::source_fingerprint(dir.path()));
    fs::remove_file(dir.path().join("src/new.rs")).unwrap();
    fs::write(dir.path().join("Cargo.toml"), "new manifest").unwrap();
    assert_ne!(original, build_script::source_fingerprint(dir.path()));
}

#[test]
fn credential_helpers_and_native_manifest_change_identity() {
    let dir = fixture();
    fs::write(dir.path().join("credential/lib.rs"), "initial").unwrap();
    let original = build_script::source_fingerprint(dir.path());
    fs::write(dir.path().join("credential/lib.rs"), "changed").unwrap();
    assert_ne!(original, build_script::source_fingerprint(dir.path()));
    fs::write(dir.path().join("credential/lib.rs"), "initial").unwrap();
    fs::write(dir.path().join("windows.manifest.xml"), "changed").unwrap();
    assert_ne!(original, build_script::source_fingerprint(dir.path()));
}
