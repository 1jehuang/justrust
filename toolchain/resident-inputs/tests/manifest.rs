use jr_resident_inputs::{MAX_FILES, MAX_TOTAL_BYTES, Manifest, capture};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "jr-inputs-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        fs::write(path.join("direct.rlib"), b"signature:one body:one").unwrap();
        Self(path)
    }
    fn get(&self, extra: &[&str]) -> Result<Option<Manifest>, String> {
        let mut args = vec!["--extern=direct=direct.rlib".to_string()];
        args.extend(extra.iter().map(|x| x.to_string()));
        capture(&args, &self.0)
    }
    fn file(&self, name: &str, bytes: &[u8]) {
        fs::write(self.0.join(name), bytes).unwrap();
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn matching_rlib_rmeta_pair_is_owned_and_both_contents_verified() {
    let f = Fixture::new();
    f.file("direct.rmeta", b"metadata");
    let args = ["--extern=direct=direct.rmeta"];
    let original = f.get(&args).unwrap().unwrap();
    f.file("direct.rmeta", b"changed!");
    assert!(original.verify().is_err());
    let current = f.get(&args).unwrap().unwrap();
    f.file("direct.rlib", b"different");
    assert!(current.verify().is_err());
    f.file("other.rmeta", b"other");
    assert!(f.get(&["--extern=direct=other.rmeta"]).is_err());
}

#[test]
fn content_identity_restored_mtime_and_touch() {
    let f = Fixture::new();
    let old = f.get(&[]).unwrap().unwrap();
    let key = old.key().unwrap();
    let path = f.0.join("direct.rlib");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    file.set_times(fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
        .unwrap();
    assert_eq!(key, f.get(&[]).unwrap().unwrap().key().unwrap());
    for bytes in [b"signature:two body:one", b"signature:two body:two"] {
        fs::write(&path, bytes).unwrap();
        file.set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        assert!(old.verify().is_err());
        assert_ne!(key, f.get(&[]).unwrap().unwrap().key().unwrap());
    }
}

#[test]
fn transitive_membership_and_replacement() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("deps")).unwrap();
    f.file("deps/trans.rmeta", b"one");
    let args = ["-Ldependency=deps"];
    let a = f.get(&args).unwrap().unwrap();
    let key = a.key().unwrap();
    f.file("deps/trans.rmeta", b"two");
    assert!(a.verify().is_err());
    assert_ne!(key, f.get(&args).unwrap().unwrap().key().unwrap());
    let b = f.get(&args).unwrap().unwrap();
    f.file("deps/added.rlib", b"x");
    assert!(b.verify().is_err());
    let c = f.get(&args).unwrap().unwrap();
    fs::remove_file(f.0.join("deps/added.rlib")).unwrap();
    assert!(c.verify().is_err());
}

#[test]
fn ordering_duplicates_relative_paths_and_serde() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("deps")).unwrap();
    f.file("second.rmeta", b"two");
    let a = f
        .get(&["--extern", "two=second.rmeta", "-L", "dependency=deps"])
        .unwrap()
        .unwrap();
    let args = vec![
        "-Ldependency=./deps".into(),
        "--extern=two=./second.rmeta".into(),
        "--extern".into(),
        "direct=deps/../direct.rlib".into(),
        "--extern=two=second.rmeta".into(),
        "-Ldependency=deps".into(),
    ];
    let b = capture(&args, &f.0).unwrap().unwrap();
    assert_eq!(a, b);
    assert_eq!(a.key().unwrap(), b.key().unwrap());
    let decoded: Manifest = serde_json::from_slice(&serde_json::to_vec(&a).unwrap()).unwrap();
    decoded.verify().unwrap();
    assert_eq!(a, decoded);
    let relative_cwd = Path::new(".");
    let abs_args = vec![format!(
        "--extern=direct={}",
        f.0.join("direct.rlib").display()
    )];
    assert_eq!(
        capture(&abs_args, relative_cwd).unwrap(),
        f.get(&[]).unwrap()
    );
}

#[test]
fn malformed_and_unsupported() {
    let f = Fixture::new();
    assert_eq!(capture(&[], &f.0).unwrap(), None);
    assert_eq!(capture(&["-Lnative=elsewhere".into()], &f.0).unwrap(), None);
    for extra in [
        vec!["--extern"],
        vec!["--extern=foo"],
        vec!["--extern==direct.rlib"],
        vec!["--extern=direct=missing.rlib"],
        vec!["--extern=direct=second.rmeta"],
        vec!["-L"],
        vec!["-Lnative=here"],
        vec!["-Lframework=here"],
        vec!["-Ldependency="],
        vec!["--extern=x=proc.so"],
    ] {
        assert!(f.get(&extra).is_err(), "{extra:?}");
    }
    f.file("proc.so", b"proc");
    assert!(f.get(&["-Ldependency=."]).is_err());
    assert!(f.get(&["--extern=p=proc.so"]).is_err());
}

#[cfg(unix)]
#[test]
fn symlinks_rejected_in_paths_and_closure() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    symlink(f.0.join("direct.rlib"), f.0.join("link.rlib")).unwrap();
    assert!(f.get(&["--extern=x=link.rlib"]).is_err());
    assert!(f.get(&["-Ldependency=."]).is_err());
    symlink(&f.0, f.0.join("dirlink")).unwrap();
    assert!(f.get(&["--extern=x=dirlink/../direct.rlib"]).is_err());
    assert!(f.get(&["-Ldependency=dirlink"]).is_err());
}

#[test]
fn file_and_byte_budgets() {
    let f = Fixture::new();
    let large = fs::File::create(f.0.join("large.rlib")).unwrap();
    large.set_len(MAX_TOTAL_BYTES + 1).unwrap();
    assert!(
        f.get(&["--extern=large=large.rlib"])
            .unwrap_err()
            .contains("byte budget")
    );
    fs::remove_file(f.0.join("large.rlib")).unwrap();
    for i in 0..MAX_FILES {
        f.file(&format!("extra{i}.rmeta"), b"");
    }
    assert!(
        f.get(&["-Ldependency=."])
            .unwrap_err()
            .contains("file budget")
    );
}

#[test]
fn tampered_manifest_and_serialization_budget() {
    let f = Fixture::new();
    let a = f.get(&[]).unwrap().unwrap();
    let mut value = serde_json::to_value(a).unwrap();
    value["schema"] = 999.into();
    let invalid: Manifest = serde_json::from_value(value.clone()).unwrap();
    assert!(invalid.verify().is_err());
    assert!(invalid.key().is_err());
    value["schema"] = jr_resident_inputs::SCHEMA.into();
    value["files"][0]["hash"] = "x".repeat(1 << 20).into();
    let invalid: Manifest = serde_json::from_value(value).unwrap();
    assert!(invalid.verify().unwrap_err().contains("budget"));
}

#[test]
fn native_static_content_and_membership_are_exact() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("native")).unwrap();
    f.file("native/libfoo.a", b"!<arch>\nbody-one");
    f.file("native/build-note", b"one");
    let args = ["-L", "native=native"];
    let a = f.get(&args).unwrap().unwrap();
    assert!(a.directories().any(|p| p == f.0.join("native")));
    assert!(a.files().any(|p| p == f.0.join("native/libfoo.a")));
    let key = a.key().unwrap();
    let path = f.0.join("native/libfoo.a");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    f.file("native/libfoo.a", b"!<arch>\nbody-two");
    fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    assert!(a.verify().is_err());
    assert_ne!(key, f.get(&args).unwrap().unwrap().key().unwrap());
    let b = f.get(&args).unwrap().unwrap();
    f.file("native/build-note", b"two");
    assert!(b.verify().is_err());
    let c = f.get(&args).unwrap().unwrap();
    f.file("native/libadded.a", b"!<arch>\n");
    assert!(c.verify().is_err());
    let d = f.get(&args).unwrap().unwrap();
    fs::remove_file(f.0.join("native/libadded.a")).unwrap();
    assert!(d.verify().is_err());
    let e = f.get(&args).unwrap().unwrap();
    let decoded: Manifest = serde_json::from_slice(&serde_json::to_vec(&e).unwrap()).unwrap();
    decoded.verify().unwrap();
    assert_eq!(e, decoded);
}

#[test]
fn native_unsupported_inputs_fail_closed() {
    for (name, bytes) in [
        ("libfoo.so", b"dynamic".as_slice()),
        ("libfoo.so.1", b"dynamic"),
        ("libfoo.dylib", b"dynamic"),
        ("libfoo.dll", b"dynamic"),
        ("libfoo.lib", b"import"),
        ("libfoo.a", b"!<thin>\nexternal"),
        ("libfoo.a", b"INPUT(/outside/archive.a)"),
    ] {
        let f = Fixture::new();
        fs::create_dir(f.0.join("native")).unwrap();
        f.file(&format!("native/{name}"), bytes);
        assert!(f.get(&["-Lnative=native"]).is_err(), "{name}");
    }
    let f = Fixture::new();
    fs::create_dir_all(f.0.join("native/nested")).unwrap();
    assert!(f.get(&["-Lnative=native"]).is_err());
}

#[cfg(unix)]
#[test]
fn native_symlinks_and_post_capture_replacement_rejected() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    fs::create_dir(f.0.join("native")).unwrap();
    f.file("native/libfoo.a", b"!<arch>\n");
    let a = f.get(&["-Lnative=native"]).unwrap().unwrap();
    fs::remove_file(f.0.join("native/libfoo.a")).unwrap();
    symlink(f.0.join("direct.rlib"), f.0.join("native/libfoo.a")).unwrap();
    assert!(a.verify().is_err());
    assert!(f.get(&["-Lnative=native"]).is_err());
    symlink(f.0.join("native"), f.0.join("alias")).unwrap();
    assert!(f.get(&["-Lnative=alias"]).is_err());
    assert!(f.get(&["-Lnative=alias/../native"]).is_err());
}

#[test]
fn native_budgets_and_empty_directory_membership() {
    let f = Fixture::new();
    fs::create_dir(f.0.join("native")).unwrap();
    let a = f.get(&["-Lnative=native"]).unwrap().unwrap();
    f.file("native/note", b"");
    assert!(a.verify().is_err());
    let large = fs::File::create(f.0.join("native/large.o")).unwrap();
    large.set_len(MAX_TOTAL_BYTES + 1).unwrap();
    assert!(
        f.get(&["-Lnative=native"])
            .unwrap_err()
            .contains("byte budget")
    );
    fs::remove_file(f.0.join("native/large.o")).unwrap();
    for i in 0..MAX_FILES {
        f.file(&format!("native/note{i}"), b"");
    }
    assert!(
        f.get(&["-Lnative=native"])
            .unwrap_err()
            .contains("file budget")
    );
}
