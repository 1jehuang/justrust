//! CLI boundary tests, including the standalone Python schema contract.
use serde_json::{Value, json};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    bundle: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn cli() -> Command {
    Command::new(env!("CARGO_BIN_EXE_justrust"))
}
fn success(output: Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
fn host() -> &'static str {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => "x86_64-unknown-linux-gnu",
        ("linux", "aarch64") => "aarch64-unknown-linux-gnu",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        _ => panic!("unsupported test host"),
    }
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "jr-bundle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let source = root.join("harness");
        fs::copy(std::env::current_exe().unwrap(), &source).unwrap();
        let messages = format!(
            "{}\n{}\n",
            json!({"reason":"compiler-artifact", "profile":{"test":true}, "executable":source}),
            json!({"reason":"build-finished", "success":true})
        );
        fs::write(root.join("messages"), messages).unwrap();
        let bundle = root.join("bundle");
        success(
            cli()
                .args(["test-bundle", "pack", "--target", host(), "--messages"])
                .arg(root.join("messages"))
                .arg("--artifact-root")
                .arg(&root)
                .arg("--out")
                .arg(&bundle)
                .output()
                .unwrap(),
        );
        Self { root, bundle }
    }
    fn manifest(&self) -> Value {
        serde_json::from_slice(&fs::read(self.bundle.join("manifest.json")).unwrap()).unwrap()
    }
    fn change(&self, f: impl FnOnce(&mut Value)) {
        let mut manifest = self.manifest();
        f(&mut manifest);
        fs::write(
            self.bundle.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        cli()
            .args(["test-bundle", "run", "--bundle"])
            .arg(&self.bundle)
            .arg("--results")
            .arg(self.root.join("result.json"))
            .arg("--")
            .args(args)
            .output()
            .unwrap()
    }
    fn rejected(&self) {
        let out = self.run(&["--exact", "bundle_fixture"]);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!self.root.join("result.json").exists());
    }
}

#[test]
fn bundle_fixture() {
    if let Some(expected) = std::env::var_os("JUSTRUST_BUNDLE_EXPECT_CWD") {
        assert_eq!(std::env::current_dir().unwrap(), PathBuf::from(expected));
    }
    assert!(
        std::env::var_os("JUSTRUST_BUNDLE_DELIBERATE_FAILURE").is_none(),
        "deliberate fixture failure"
    );
}

#[test]
fn runs_and_never_changes_manifest_or_overwrites_results() {
    let f = Fixture::new();
    let before = fs::read(f.bundle.join("manifest.json")).unwrap();
    success(f.run(&["--exact", "bundle_fixture"]));
    success(
        cli()
            .env("JUSTRUST_BUNDLE_EXPECT_CWD", &f.root)
            .args(["test-bundle", "run", "--bundle"])
            .arg(&f.bundle)
            .arg("--results")
            .arg(f.root.join("cwd-result.json"))
            .arg("--cwd")
            .arg(&f.root)
            .args(["--", "--exact", "bundle_fixture"])
            .output()
            .unwrap(),
    );
    let result: Value =
        serde_json::from_slice(&fs::read(f.root.join("result.json")).unwrap()).unwrap();
    assert_eq!(result["test_status"], "passed");
    assert_eq!(before, fs::read(f.bundle.join("manifest.json")).unwrap());
    assert_eq!(f.run(&[]).status.code(), Some(2));
    let failed = cli()
        .env("JUSTRUST_BUNDLE_DELIBERATE_FAILURE", "1")
        .args(["test-bundle", "run", "--bundle"])
        .arg(&f.bundle)
        .arg("--results")
        .arg(f.root.join("failed.json"))
        .args(["--", "--exact", "bundle_fixture"])
        .output()
        .unwrap();
    assert_eq!(failed.status.code(), Some(1));
    let result: Value =
        serde_json::from_slice(&fs::read(f.root.join("failed.json")).unwrap()).unwrap();
    assert_eq!(result["test_status"], "failed");
    assert_ne!(result["tests"][0]["exit_code"], 0);
}

#[test]
fn strict_manifest_validation() {
    for change in [
        ("schema_version", json!(true)),
        ("schema_version", json!(1.0)),
        ("schema_version", json!(2)),
        ("extra", json!(0)),
        ("build_status", json!("failed")),
        ("test_status", json!("passed")),
        ("target", json!("invented")),
        ("tests", json!([])),
    ] {
        let f = Fixture::new();
        f.change(|v| v[change.0] = change.1);
        f.rejected();
    }
    for path in [
        "../escape",
        "bin/../escape",
        "bin//file",
        "bin/.",
        "bin/..",
        "bin/a:b",
        "bin/a\\b",
        "/bin/file",
    ] {
        let f = Fixture::new();
        f.change(|v| v["tests"][0]["path"] = json!(path));
        f.rejected();
    }
    for (key, value) in [
        ("size", json!(-1)),
        ("size", json!(true)),
        ("sha256", json!("a".repeat(63))),
        ("extra", json!(0)),
    ] {
        let f = Fixture::new();
        f.change(|v| v["tests"][0][key] = value);
        f.rejected();
    }
    let f = Fixture::new();
    f.change(|v| {
        let test = v["tests"][0].clone();
        v["tests"].as_array_mut().unwrap().push(test);
    });
    f.rejected();
    let f = Fixture::new();
    let manifest = f.manifest();
    fs::write(
        f.bundle.join("manifest.json"),
        serde_json::to_vec(&json!([
            1,
            host(),
            "compiled",
            "not-tested",
            manifest["tests"]
        ]))
        .unwrap(),
    )
    .unwrap();
    f.rejected();
    let f = Fixture::new();
    f.change(|v| {
        let t = &v["tests"][0];
        v["tests"] = json!([[t["path"], t["sha256"], t["size"]]]);
    });
    f.rejected();
    let f = Fixture::new();
    let text = fs::read_to_string(f.bundle.join("manifest.json"))
        .unwrap()
        .replace(
            "\"schema_version\": 1",
            "\"schema_version\": 1, \"schema_version\": 1",
        );
    fs::write(f.bundle.join("manifest.json"), text).unwrap();
    f.rejected();
    let f = Fixture::new();
    fs::write(
        f.bundle.join("manifest.json"),
        vec![b' '; 4 * 1024 * 1024 + 1],
    )
    .unwrap();
    f.rejected();
}

#[test]
fn validates_every_artifact_before_execution() {
    let f = Fixture::new();
    f.change(|v| {
        let mut test = v["tests"][0].clone();
        test["path"] = json!("bin/second");
        v["tests"].as_array_mut().unwrap().push(test);
    });
    fs::write(f.bundle.join("bin/second"), b"corrupt").unwrap();
    f.rejected();
}

#[test]
fn refuses_non_test_modes_and_wrong_host() {
    for arg in ["--list", "--list=true", "--help", "-h", "-qh", "--bench"] {
        let f = Fixture::new();
        assert_eq!(f.run(&[arg]).status.code(), Some(2));
        assert!(!f.root.join("result.json").exists());
    }
    let f = Fixture::new();
    f.change(|v| {
        v["target"] = json!(if std::env::consts::OS == "windows" {
            "aarch64-apple-darwin"
        } else {
            "x86_64-pc-windows-msvc"
        })
    });
    f.rejected();
}

#[cfg(unix)]
#[test]
fn refuses_symlinks_and_bundle_local_results() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let test = f.manifest()["tests"][0]["path"]
        .as_str()
        .unwrap()
        .to_owned();
    fs::rename(f.bundle.join(&test), f.root.join("real")).unwrap();
    symlink(f.root.join("real"), f.bundle.join(&test)).unwrap();
    f.rejected();
    let f = Fixture::new();
    fs::rename(f.bundle.join("bin"), f.root.join("real-bin")).unwrap();
    symlink(f.root.join("real-bin"), f.bundle.join("bin")).unwrap();
    f.rejected();
    let f = Fixture::new();
    symlink(&f.bundle, f.root.join("alias")).unwrap();
    assert_eq!(
        cli()
            .args(["test-bundle", "run", "--bundle"])
            .arg(f.root.join("alias"))
            .arg("--results")
            .arg(f.root.join("result.json"))
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    let f = Fixture::new();
    assert_eq!(
        cli()
            .args(["test-bundle", "run", "--bundle"])
            .arg(&f.bundle)
            .arg("--results")
            .arg(f.bundle.join("result.json"))
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
    symlink(f.root.join("missing"), f.root.join("result.json")).unwrap();
    assert_eq!(f.run(&[]).status.code(), Some(2));
}

#[test]
fn requires_successful_cargo_stream_and_contained_artifacts() {
    for content in [
        json!({"reason":"build-finished","success":false}).to_string(),
        "{}".into(),
        "[]".into(),
        " ".repeat(4 * 1024 * 1024 + 1),
    ] {
        let f = Fixture::new();
        fs::write(f.root.join("messages"), content).unwrap();
        assert_eq!(
            cli()
                .args(["test-bundle", "pack", "--target", host(), "--messages"])
                .arg(f.root.join("messages"))
                .arg("--artifact-root")
                .arg(&f.root)
                .arg("--out")
                .arg(f.root.join("other"))
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
        assert!(!f.root.join("other").exists());
    }
    let f = Fixture::new();
    let valid = fs::read_to_string(f.root.join("messages")).unwrap();
    for content in [
        format!("{valid}{{\"reason\":\"build-finished\",\"success\":false}}\n"),
        valid.replace("\"success\":true", "\"success\":false,\"success\":true"),
        valid.lines().next().unwrap().to_owned(),
    ] {
        fs::write(f.root.join("messages"), content).unwrap();
        assert_eq!(
            cli()
                .args(["test-bundle", "pack", "--target", host(), "--messages"])
                .arg(f.root.join("messages"))
                .arg("--artifact-root")
                .arg(&f.root)
                .arg("--out")
                .arg(f.root.join("other"))
                .output()
                .unwrap()
                .status
                .code(),
            Some(2)
        );
        assert!(!f.root.join("other").exists());
    }
    fs::write(f.root.join("messages"), valid).unwrap();
    fs::create_dir(f.root.join("empty")).unwrap();
    assert_eq!(
        cli()
            .args(["test-bundle", "pack", "--target", host(), "--messages"])
            .arg(f.root.join("messages"))
            .arg("--artifact-root")
            .arg(f.root.join("empty"))
            .arg("--out")
            .arg(f.root.join("other"))
            .output()
            .unwrap()
            .status
            .code(),
        Some(2)
    );
}

#[test]
fn python_interoperability_both_directions() {
    let f = Fixture::new();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/cross-test-bundle.py");
    let python = std::env::var("PYTHON").unwrap_or_else(|_| "python3".into());
    success(
        Command::new(&python)
            .arg(&helper)
            .arg("run")
            .arg("--bundle")
            .arg(&f.bundle)
            .arg("--results")
            .arg(f.root.join("python-result.json"))
            .args(["--", "--exact", "bundle_fixture"])
            .output()
            .unwrap(),
    );
    let python_bundle = f.root.join("python-bundle");
    success(
        Command::new(&python)
            .arg(&helper)
            .args(["bundle", "--target", host(), "--messages"])
            .arg(f.root.join("messages"))
            .arg("--artifact-root")
            .arg(&f.root)
            .arg("--out")
            .arg(&python_bundle)
            .output()
            .unwrap(),
    );
    success(
        cli()
            .args(["test-bundle", "run", "--bundle"])
            .arg(&python_bundle)
            .arg("--results")
            .arg(f.root.join("native-result.json"))
            .args(["--", "--exact", "bundle_fixture"])
            .output()
            .unwrap(),
    );
    assert_eq!(
        f.manifest(),
        serde_json::from_slice::<Value>(&fs::read(python_bundle.join("manifest.json")).unwrap())
            .unwrap()
    );
}
