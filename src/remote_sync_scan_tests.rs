//! Equivalence coverage and an opt-in, narrowly scoped walker diagnostic.
use super::*;
use std::ffi::OsStr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

struct ScanTree(PathBuf);

impl ScanTree {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "jr-sync-scan-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        // create_dir deliberately fails rather than reusing stale fixtures.
        std::fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn file(&self, relative: impl AsRef<Path>) {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"fixture\n").unwrap();
    }
}

impl Drop for ScanTree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// Preserve the pre-change walker independently of walk_matching so the
// equivalence checks and diagnostic do not compare two modes of one walker.
fn legacy_walk(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![PathBuf::new()];
    while let Some(rel) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(root.join(&rel)) else {
            continue;
        };
        for entry in rd.flatten() {
            let relative = rel.join(entry.file_name());
            if skip(&relative) {
                continue;
            }
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => stack.push(relative),
                Ok(kind) if kind.is_file() => out.push(relative),
                _ => {}
            }
        }
    }
    out
}

fn legacy_locks(root: &Path) -> Vec<PathBuf> {
    legacy_walk(root)
        .into_iter()
        .filter(|path| path.file_name() == Some(OsStr::new("Cargo.lock")))
        .collect()
}

fn filtered_locks(root: &Path) -> Vec<PathBuf> {
    walk_matching(root, Some(OsStr::new("Cargo.lock"))).unwrap()
}

fn assert_locks(root: &Path, expected: Vec<PathBuf>) {
    let mut legacy_files = legacy_walk(root);
    let mut current_files = walk(root).unwrap();
    legacy_files.sort();
    current_files.sort();
    assert_eq!(current_files, legacy_files, "unfiltered walk changed files");
    let mut legacy = legacy_locks(root);
    let mut filtered = filtered_locks(root);
    legacy.sort();
    filtered.sort();
    let mut expected = expected;
    expected.sort();
    assert_eq!(filtered, legacy, "filtered walk differs from walk+filter");
    assert_eq!(filtered, expected);
}

#[test]
fn filtered_walk_nested_ignored_and_pruned_directories() {
    let tree = ScanTree::new();
    tree.file(".gitignore");
    std::fs::write(tree.0.join(".gitignore"), "ignored/\n").unwrap();
    let expected = [
        "Cargo.lock",
        "ignored/Cargo.lock",
        "ignored/vendor/nested/Cargo.lock",
        "ignored/container/Cargo.lock/inside/Cargo.lock",
        "target-like/Cargo.lock",
    ];
    for path in expected {
        tree.file(path);
    }
    for path in [
        "ignored/src/lib.rs",
        "ignored/vendor/Cargo.lock.backup",
        "ignored/vendor/cargo.lock",
        "target/Cargo.lock",
        ".git/Cargo.lock",
        ".justrust/Cargo.lock",
        "ignored/vendor/target/Cargo.lock",
        "ignored/vendor/.git/Cargo.lock",
        "ignored/vendor/.justrust/Cargo.lock",
    ] {
        tree.file(path);
    }
    assert_locks(&tree.0, expected.into_iter().map(PathBuf::from).collect());
    assert_locks(
        &tree.0.join("ignored"),
        vec![
            "Cargo.lock".into(),
            "vendor/nested/Cargo.lock".into(),
            "container/Cargo.lock/inside/Cargo.lock".into(),
        ],
    );
}

#[test]
fn filtered_walk_deep_paths_and_fresh_create_delete() {
    let tree = ScanTree::new();
    assert_locks(&tree.0, vec![]);
    let mut deep = PathBuf::new();
    for level in 0..96 {
        deep.push(format!("d{level}"));
        tree.file(deep.join("unrelated.rs"));
    }
    let lock = deep.join("Cargo.lock");
    tree.file(&lock);
    assert_locks(&tree.0, vec![lock.clone()]);
    std::fs::remove_file(tree.0.join(&lock)).unwrap();
    assert_locks(&tree.0, vec![]);
    tree.file("newly-created/nested/Cargo.lock");
    assert_locks(&tree.0, vec!["newly-created/nested/Cargo.lock".into()]);
    std::fs::remove_dir_all(tree.0.join("newly-created")).unwrap();
    assert_locks(&tree.0, vec![]);
    assert_locks(&tree.0.join("missing"), vec![]);
}

#[cfg(unix)]
#[test]
fn filtered_walk_does_not_follow_file_or_directory_symlinks() {
    use std::os::unix::fs::symlink;
    let tree = ScanTree::new();
    let outside = ScanTree::new();
    outside.file("Cargo.lock");
    tree.file("real/Cargo.lock");
    tree.file("ordinary.txt");
    symlink(tree.0.join("real/Cargo.lock"), tree.0.join("Cargo.lock")).unwrap();
    symlink(&outside.0, tree.0.join("linked-directory")).unwrap();
    symlink(tree.0.join("real"), tree.0.join("linked-internal")).unwrap();
    symlink(&tree.0, tree.0.join("cycle")).unwrap();
    symlink(tree.0.join("missing"), tree.0.join("dangling")).unwrap();
    assert_locks(&tree.0, vec!["real/Cargo.lock".into()]);
}

// APFS rejects non-UTF-8 names (EILSEQ).
#[cfg(all(unix, not(target_vendor = "apple")))]
#[test]
fn filtered_walk_preserves_non_utf8_paths_and_exact_names() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let tree = ScanTree::new();
    let dir = PathBuf::from(OsString::from_vec(b"ignored-\xff".to_vec()));
    let lock = dir.join("nested/Cargo.lock");
    tree.file(&lock);
    tree.file(dir.join(OsString::from_vec(b"Cargo.lock\xff".to_vec())));
    tree.file(dir.join("target/Cargo.lock"));
    tree.file(OsString::from_vec(b"unrelated-\xfe".to_vec()));
    assert_locks(&tree.0, vec![lock]);
}

#[test]
#[ignore = "comparative filesystem diagnostic, not a broad build-speed benchmark"]
fn diagnostic_filtered_walk_vs_legacy_walk_and_filter() {
    let tree = ScanTree::new();
    let mut expected = Vec::new();
    // Model ignored vendored sources and generated assets, dominated by files
    // that cannot be lockfiles, with lockfiles spread across nested packages.
    for package in 0..80 {
        let package = PathBuf::from(format!("ignored/vendor/package-{package}"));
        let lock = package.join("Cargo.lock");
        tree.file(&lock);
        expected.push(lock);
        for file in 0..160 {
            tree.file(package.join(format!("src/module-{}/file-{file}.rs", file % 8)));
        }
        tree.file(package.join("target/debug/Cargo.lock"));
    }
    assert_locks(&tree.0, expected);
    // Warm both paths before measurement. Alternate which path runs first to
    // avoid giving either implementation a systematic cache-order advantage.
    let _ = legacy_locks(&tree.0);
    let _ = filtered_locks(&tree.0);
    let mut legacy_times = Vec::new();
    let mut filtered_times = Vec::new();
    for round in 0..16 {
        let mut legacy_result = Vec::new();
        let mut filtered_result = Vec::new();
        for filtered in [round % 2 == 0, round % 2 != 0] {
            let start = Instant::now();
            let result = if filtered {
                filtered_locks(&tree.0)
            } else {
                legacy_locks(&tree.0)
            };
            let elapsed = start.elapsed();
            if filtered {
                filtered_times.push(elapsed);
                filtered_result = result;
            } else {
                legacy_times.push(elapsed);
                legacy_result = result;
            }
        }
        legacy_result.sort();
        filtered_result.sort();
        assert_eq!(legacy_result, filtered_result);
        assert_eq!(filtered_result.len(), 80);
        std::hint::black_box((&legacy_result, &filtered_result));
    }
    fn median(samples: &mut [Duration]) -> Duration {
        samples.sort();
        let middle = samples.len() / 2;
        (samples[middle - 1] + samples[middle]) / 2
    }
    eprintln!(
        "walker-only diagnostic: 80 lockfiles, 12800 unrelated regular files, \
         16 alternating rounds; legacy walk+filter median {:?}; \
         filtered walk median {:?}; not a broad build-speed measurement",
        median(&mut legacy_times),
        median(&mut filtered_times),
    );
}
