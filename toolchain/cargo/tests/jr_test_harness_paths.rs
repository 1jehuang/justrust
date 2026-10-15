//! Native Cargo's compiler storage layout must not leak into test execution.
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::Value;
use tempfile::TempDir;

fn cargo(root: &Path, args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_cargo"))
        .current_dir(root)
        .args(args)
        .args(["--offline", "--target-dir"])
        .arg(root.join("artifacts"))
        .args(["--config", "build.build-dir=\"private-build\""])
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "cargo {args:?}:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn fixture() -> TempDir {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "harness-path-contract"
version = "0.0.0"
edition = "2024"

[[test]]
name = "custom"
harness = false
"#,
    )
    .unwrap();
    let assertion = r#"
    let exe = std::env::current_exe().unwrap();
    assert_eq!(exe.parent().unwrap().file_name().unwrap(), "deps", "{exe:?}");
    let filename = exe.file_stem().unwrap().to_str().unwrap();
    let (_, hash) = filename.rsplit_once('-').unwrap();
    assert_eq!(hash.len(), 16, "{exe:?}");
    assert!(hash.bytes().all(|byte| byte.is_ascii_hexdigit()), "{exe:?}");
"#;
    for file in ["src/lib.rs", "tests/integration.rs"] {
        std::fs::write(
            root.join(file),
            format!("#[test]\nfn marker() {{\n{assertion}\n}}\n"),
        )
        .unwrap();
    }
    std::fs::write(
        root.join("tests/custom.rs"),
        format!("fn main() {{\n{assertion}\n}}\n"),
    )
    .unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    temp
}

fn artifacts(output: &Output) -> Vec<Value> {
    String::from_utf8(output.stdout.clone())
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|row| row["reason"] == "compiler-artifact" && row["profile"]["test"] == true)
        .collect()
}

#[test]
fn public_test_paths_survive_private_build_storage_and_warm_relink() {
    for release in [false, true] {
        let temp = fixture();
        let root = temp.path();
        let mut args = vec![
            "test",
            "--lib",
            "--test",
            "integration",
            "--test",
            "custom",
            "--no-run",
            "--message-format=json",
        ];
        if release {
            args.push("--release");
        }
        let output = cargo(root, &args);
        let cold = artifacts(&output);
        assert_eq!(cold.len(), 3);
        let mut paths = Vec::new();
        for row in &cold {
            let path = PathBuf::from(row["executable"].as_str().unwrap());
            assert_eq!(path.parent().unwrap().file_name().unwrap(), "deps");
            assert!(path.starts_with(root.join("artifacts")));
            assert!(
                row["filenames"]
                    .as_array()
                    .unwrap()
                    .contains(&row["executable"])
            );
            assert!(path.is_file());
            let result = Command::new(&path)
                .args(["--exact", "marker"])
                .output()
                .unwrap();
            assert!(result.status.success(), "{path:?}: {result:?}");
            paths.push(path);
        }
        // Compiler storage remains private, not redirected to the public deps tree.
        assert!(root.join("private-build").exists());
        for path in &paths {
            std::fs::remove_file(path).unwrap();
        }
        let warm = artifacts(&cargo(root, &args));
        assert_eq!(warm.len(), 3);
        assert!(warm.iter().all(|row| row["fresh"] == true));
        assert!(paths.iter().all(|path| path.is_file()));
        // Exercise Cargo's own execution collection too, not just JSON/no-run.
        args.retain(|arg| *arg != "--no-run");
        cargo(root, &args);
    }
}

#[test]
fn ordinary_binary_execution_path_is_not_a_test_path() {
    let temp = fixture();
    let rows = cargo(
        temp.path(),
        &[
            "build",
            "--bin",
            "harness-path-contract",
            "--message-format=json",
        ],
    );
    let output = String::from_utf8(rows.stdout).unwrap();
    let executable = output
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|row| {
            row["reason"] == "compiler-artifact"
                && row["target"]["kind"] == serde_json::json!(["bin"])
        })
        .unwrap();
    let path = Path::new(executable["executable"].as_str().unwrap());
    assert_eq!(path.parent().unwrap().file_name().unwrap(), "debug");
}
