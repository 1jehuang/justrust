//! Real Cargo metadata and artifact server regressions. No hosted connection.
#![cfg(target_os = "linux")]

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!("jr-artifact-{name}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn serve_command(binary: &Path, root: &Path) -> Command {
    let mut command = Command::new(binary);
    command
        .arg("__artifacts")
        .env("JUSTRUST_HOME", root.join("home"))
        .env_remove("JUSTRUST_REAL_CARGO")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn request(root: &Path) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "cwd": root,
        "args": ["build", "--release", "--target", "x86_64-unknown-linux-gnu"],
        "start": 0.0,
        "have": []
    }))
    .unwrap()
}

#[test]
fn artifact_metadata_survives_atomic_replacement_of_running_client() {
    let root = scratch("replaced-exe");
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"artifact-replacement-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"lib.rs\"\n").unwrap();
    std::fs::write(root.join("lib.rs"), "pub fn fixture() {}\n").unwrap();
    let binary = root.join("justrust");
    let replacement = root.join("justrust-new");
    std::fs::copy(env!("CARGO_BIN_EXE_justrust"), &binary).unwrap();
    let mut child = serve_command(&binary, &root).spawn().unwrap();
    // serve is waiting for EOF. Like cargo install, rename a new binary over
    // the old inode while that inode is still executing. /proc/self/exe now
    // reports the old path with " (deleted)" and canonicalize fails.
    std::fs::copy(env!("CARGO_BIN_EXE_justrust"), &replacement).unwrap();
    std::fs::rename(&replacement, &binary).unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&request(&root))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let frames: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(frames[0]["t"], "Hello");
    assert_eq!(
        frames[0]["dir"],
        root.join("target/x86_64-unknown-linux-gnu/release")
            .display()
            .to_string()
    );
    assert_eq!(frames[1]["t"], "End");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn artifact_metadata_preserves_cargo_failure_diagnostics() {
    let root = scratch("metadata-error");
    std::fs::write(root.join("Cargo.toml"), "[package\n").unwrap();
    let mut child = serve_command(Path::new(env!("CARGO_BIN_EXE_justrust")), &root)
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&request(&root))
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("could not resolve the target dir"),
        "{stderr}"
    );
    assert!(stderr.contains("Cargo metadata for"), "{stderr}");
    assert!(stderr.contains("Cargo.toml"), "{stderr}");
    assert!(stderr.contains("error:"), "{stderr}");
    assert!(stderr.contains("[package"), "{stderr}");
    std::fs::remove_dir_all(root).unwrap();
}
