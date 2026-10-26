//! Real Cargo metadata and artifact server regressions. No hosted connection.
#![cfg(target_os = "linux")]

use std::io::Write;
use std::io::{BufRead, Read};
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
        .env("CARGO_TERM_COLOR", "never")
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
    // Use Cargo itself as the diagnostic oracle: TOML parser wording and
    // source excerpts change between toolchains, but the causal error must
    // survive unchanged in the artifact server's contextual failure.
    let cargo = Command::new("cargo")
        .current_dir(&root)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
        ])
        .env_remove("JUSTRUST_REAL_CARGO")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC")
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .unwrap();
    assert!(!cargo.status.success(), "{cargo:?}");
    let cargo_stderr = String::from_utf8(cargo.stderr).unwrap();
    assert!(cargo_stderr.contains("Cargo.toml"), "{cargo_stderr}");
    assert!(cargo_stderr.contains("error:"), "{cargo_stderr}");
    assert!(cargo_stderr.contains("expected `]`"), "{cargo_stderr}");
    assert!(stderr.contains(cargo_stderr.trim()), "{stderr}");
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn compile_only_transports_selected_cargo_tests_without_execution() {
    use std::os::unix::fs::PermissionsExt;
    let root = scratch("selected-tests");
    let binary = Path::new(env!("CARGO_BIN_EXE_justrust"));
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"selected-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"lib.rs\"\n").unwrap();
    std::fs::write(
        root.join("lib.rs"),
        "#[test] fn only_selected() { std::fs::write(\"test-ran\", b\"yes\").unwrap(); }\n",
    )
    .unwrap();
    let output = Command::new(binary)
        .args([
            "test",
            "--local",
            "--no-run",
            "--message-format=json",
            "--lib",
        ])
        .current_dir(&root)
        .env("JUSTRUST_HOME", root.join("home"))
        .env("JUSTRUST_NO_TELEMETRY", "1")
        .env("JUSTRUST_NO_UPDATE", "1")
        .env("JUSTRUST_TOOLCHAIN", "system")
        .env_remove("JUSTRUST_REMOTE_REQUIRED")
        .env_remove("JUSTRUST_TOOLCHAIN_REQUIRED")
        .env_remove("JUSTRUST_ON_REMOTE_AGENT")
        .env_remove("JUSTRUST_RUN_ID_OVERRIDE")
        .env_remove("JUSTRUST_RUN_DIR")
        .env_remove("JUSTRUST_DISABLE")
        .env_remove("JUSTRUST_REAL_CARGO")
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(!root.join("test-ran").exists());
    let artifacts: Vec<serde_json::Value> = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|v| {
            v["reason"] == "compiler-artifact"
                && v["profile"]["test"] == true
                && v["executable"].is_string()
        })
        .collect();
    assert_eq!(artifacts.len(), 1);
    let original = PathBuf::from(artifacts[0]["executable"].as_str().unwrap());
    let stale = original
        .parent()
        .unwrap()
        .join("stale-unselected-0123456789abcdef");
    std::fs::copy(&original, &stale).unwrap();
    let runs = std::fs::read_dir(root.join("home/runs"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("summary.json").is_file())
        .collect::<Vec<_>>();
    assert_eq!(runs.len(), 1);
    let id = runs[0].file_name().unwrap().to_str().unwrap();
    let mut child = serve_command(binary, &root).spawn().unwrap();
    let request = serde_json::json!({"cwd": root, "args": ["test","--no-run","--lib"], "test_run": id, "start":0.0,"have":[]});
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(&request).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    let mut reader = std::io::BufReader::new(output.stdout.as_slice());
    let mut downloaded = Vec::new();
    let mut selected = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let frame: serde_json::Value = serde_json::from_str(&line).unwrap();
        match frame["t"].as_str().unwrap() {
            "Hello" => (),
            "Selected" => selected = frame["artifacts"].as_array().unwrap().clone(),
            "File" => {
                let name = frame["name"].as_str().unwrap();
                assert!(!name.contains("stale-unselected"));
                let mut data = vec![0; frame["len"].as_u64().unwrap() as usize];
                reader.read_exact(&mut data).unwrap();
                let local = root.join("download").join(name);
                std::fs::create_dir_all(local.parent().unwrap()).unwrap();
                let mut decoder = Command::new("zstd")
                    .args(["-q", "-d", "-f", "-o"])
                    .arg(&local)
                    .stdin(Stdio::piped())
                    .spawn()
                    .unwrap();
                decoder.stdin.take().unwrap().write_all(&data).unwrap();
                assert!(decoder.wait().unwrap().success());
                std::fs::set_permissions(&local, std::fs::Permissions::from_mode(0o755)).unwrap();
                downloaded.push(local);
            }
            "End" => break,
            other => panic!("unexpected artifact frame {other}"),
        }
    }
    assert_eq!(selected.len(), 1);
    assert_eq!(downloaded.len(), 1);
    assert_eq!(
        std::fs::read(&original).unwrap(),
        std::fs::read(&downloaded[0]).unwrap()
    );
    assert!(
        !root.join("test-ran").exists(),
        "compile or transfer executed the test"
    );
    let test = Command::new(&downloaded[0])
        .current_dir(&root)
        .args(["--exact", "only_selected"])
        .output()
        .unwrap();
    assert!(test.status.success(), "{test:?}");
    assert!(
        root.join("test-ran").exists(),
        "downloaded test did not run locally"
    );
    std::fs::remove_dir_all(root).unwrap();
}
