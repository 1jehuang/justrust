use tempfile::TempDir;

use crate::utils::build_stamp::BuildStamp;

#[test]
#[should_panic(expected = "prefix can not start or end with '.'")]
fn test_with_invalid_prefix() {
    let dir = TempDir::new().unwrap();
    BuildStamp::new(dir.path()).with_prefix(".invalid");
}

#[test]
#[should_panic(expected = "prefix can not start or end with '.'")]
fn test_with_invalid_prefix2() {
    let dir = TempDir::new().unwrap();
    BuildStamp::new(dir.path()).with_prefix("invalid.");
}

#[test]
fn test_is_up_to_date() {
    let dir = TempDir::new().unwrap();

    let mut build_stamp = BuildStamp::new(dir.path()).add_stamp("v1.0.0");
    build_stamp.write().unwrap();

    assert!(
        build_stamp.is_up_to_date(),
        "Expected stamp file to be up-to-date, but contents do not match the expected value."
    );

    build_stamp.stamp = "dummy value".to_owned();
    assert!(
        !build_stamp.is_up_to_date(),
        "Stamp should no longer be up-to-date as we changed its content right above."
    );

    build_stamp.remove().unwrap();
}

#[test]
fn test_with_prefix() {
    let dir = TempDir::new().unwrap();

    let stamp = BuildStamp::new(dir.path()).add_stamp("v1.0.0");
    assert_eq!(stamp.path.file_name().unwrap(), ".stamp");

    let stamp = stamp.with_prefix("test");
    let expected_filename = ".test-stamp";
    assert_eq!(stamp.path.file_name().unwrap(), expected_filename);

    let stamp = stamp.with_prefix("extra-prefix");
    let expected_filename = ".extra-prefix-test-stamp";
    assert_eq!(stamp.path.file_name().unwrap(), expected_filename);
}

fn source_hash(root: &std::path::Path) -> Vec<u8> {
    use sha2::Digest;
    let mut hasher = sha2::Sha256::new();
    super::hash_source_tree(root, root, &mut hasher);
    hasher.finalize().to_vec()
}

#[test]
fn source_archive_stamp_tracks_contents_not_timestamps() {
    let dir = TempDir::new().unwrap();
    let file = dir.path().join("source.cpp");
    std::fs::write(&file, "original").unwrap();
    let original = source_hash(dir.path());
    std::fs::write(&file, "modified").unwrap();
    assert_ne!(original, source_hash(dir.path()));
    std::fs::write(&file, "original").unwrap();
    assert_eq!(original, source_hash(dir.path()));
    std::fs::rename(&file, dir.path().join("renamed.cpp")).unwrap();
    assert_ne!(original, source_hash(dir.path()));
}

#[test]
fn source_archive_stamp_is_order_independent_and_ignores_git_metadata() {
    let a = TempDir::new().unwrap();
    let b = TempDir::new().unwrap();
    for (dir, files) in [(a.path(), ["a", "b"]), (b.path(), ["b", "a"])] {
        for name in files {
            std::fs::write(dir.join(name), name).unwrap();
        }
    }
    std::fs::create_dir(a.path().join(".git")).unwrap();
    std::fs::write(a.path().join(".git/HEAD"), "irrelevant").unwrap();
    assert_eq!(source_hash(a.path()), source_hash(b.path()));
}

#[test]
#[cfg(unix)]
fn source_archive_stamp_tracks_symlink_targets_without_following_them() {
    let dir = TempDir::new().unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink("missing-a", &link).unwrap();
    let original = source_hash(dir.path());
    std::fs::remove_file(&link).unwrap();
    std::os::unix::fs::symlink("missing-b", &link).unwrap();
    assert_ne!(original, source_hash(dir.path()));
}
