#![allow(clippy::disallowed_types)] // Exercise the public utility with a standard caller.

use cargo_util::paths::{mtime_recursive, mtime_recursive_with_cache};
use filetime::{FileTime, set_file_mtime};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

fn tree(root: &Path) {
    fs::create_dir_all(root.join("a/b")).unwrap();
    fs::create_dir_all(root.join("empty")).unwrap();
    fs::write(root.join("a/b/input"), "input").unwrap();
    set_file_mtime(
        root.join("a/b/input"),
        FileTime::from_unix_time(2_000_000_000, 0),
    )
    .unwrap();
}

#[test]
fn caches_complete_subtrees_but_not_files() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let mut cache = HashMap::new();
    assert_eq!(
        mtime_recursive_with_cache(tmp.path(), &mut cache).unwrap(),
        mtime_recursive(tmp.path()).unwrap()
    );
    for relative in ["", "a", "a/b", "empty"] {
        let path = tmp.path().join(relative);
        assert_eq!(cache.get(&path), Some(&mtime_recursive(&path).unwrap()));
    }
    assert!(!cache.contains_key(&tmp.path().join("a/b/input")));
}

#[test]
fn subtree_maxima_match_independent_walks() {
    let tmp = tempfile::tempdir().unwrap();
    for (index, relative) in ["left/deep", "middle", "right/deeper/deepest"]
        .iter()
        .enumerate()
    {
        let directory = tmp.path().join(relative);
        fs::create_dir_all(&directory).unwrap();
        for file in 0..5 {
            let path = directory.join(format!("file{file}"));
            fs::write(&path, "input").unwrap();
            set_file_mtime(
                path,
                FileTime::from_unix_time(2_000_000_000 + (index * 7 + file) as i64, 0),
            )
            .unwrap();
        }
    }
    let mut cache = HashMap::new();
    mtime_recursive_with_cache(tmp.path(), &mut cache).unwrap();
    for (path, observed) in &cache {
        assert_eq!(
            *observed,
            mtime_recursive(path).unwrap(),
            "{}",
            path.display()
        );
    }
    let file = tmp.path().join("middle/file0");
    assert_eq!(
        mtime_recursive_with_cache(&file, &mut cache).unwrap(),
        mtime_recursive(&file).unwrap()
    );
}

#[test]
fn next_invocation_detects_changes_and_deletions() {
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    let old = mtime_recursive_with_cache(tmp.path(), &mut HashMap::new()).unwrap();
    let input = tmp.path().join("a/b/input");
    set_file_mtime(&input, FileTime::from_unix_time(2_000_000_001, 0)).unwrap();
    let new = mtime_recursive_with_cache(tmp.path(), &mut HashMap::new()).unwrap();
    assert!(new > old);
    fs::remove_file(input).unwrap();
    assert_eq!(
        mtime_recursive_with_cache(tmp.path(), &mut HashMap::new()).unwrap(),
        mtime_recursive(tmp.path()).unwrap()
    );
    assert!(mtime_recursive_with_cache(&tmp.path().join("missing"), &mut HashMap::new()).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_and_loops_fall_back_without_publishing_subtrees() {
    use std::os::unix::fs::symlink;
    let tmp = tempfile::tempdir().unwrap();
    tree(tmp.path());
    symlink(tmp.path(), tmp.path().join("a/loop")).unwrap();
    symlink("missing", tmp.path().join("dangling")).unwrap();
    let mut cache = HashMap::new();
    assert_eq!(
        mtime_recursive_with_cache(tmp.path(), &mut cache).unwrap(),
        mtime_recursive(tmp.path()).unwrap()
    );
    assert!(cache.is_empty());
    let alias = tmp.path().join("alias");
    symlink(tmp.path().join("empty"), &alias).unwrap();
    assert_eq!(
        mtime_recursive_with_cache(&alias, &mut cache).unwrap(),
        mtime_recursive(&alias).unwrap()
    );
    assert!(cache.is_empty());
}

#[test]
#[ignore = "focused before/after measurement, run with --ignored --nocapture"]
fn benchmark_overlapping_roots() {
    use std::time::Instant;
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..48 {
        let dir = tmp.path().join(format!("d{i}/nested"));
        fs::create_dir_all(&dir).unwrap();
        for j in 0..64 {
            fs::write(dir.join(format!("f{j}")), "input").unwrap();
        }
    }
    let mut roots = vec![tmp.path().to_path_buf()];
    for i in 0..48 {
        roots.push(tmp.path().join(format!("d{i}")));
        roots.push(tmp.path().join(format!("d{i}/nested")));
    }
    for (scenario, roots) in [("single", &roots[..1]), ("overlapping", &roots[..])] {
        for trial in 0..12 {
            for cached in if trial % 2 == 0 {
                [false, true]
            } else {
                [true, false]
            } {
                let start = Instant::now();
                let mut cache = HashMap::new();
                for root in roots {
                    let result = if cached {
                        mtime_recursive_with_cache(root, &mut cache).unwrap()
                    } else {
                        *cache
                            .entry(root.clone())
                            .or_insert_with(|| mtime_recursive(root).unwrap())
                    };
                    std::hint::black_box(result);
                }
                println!(
                    "{{\"scenario\":\"{scenario}\",\"trial\":{trial},\"cached\":{cached},\"elapsed_ns\":{}}}",
                    start.elapsed().as_nanos()
                );
            }
        }
    }
}
