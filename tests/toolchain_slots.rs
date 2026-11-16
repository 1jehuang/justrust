//! CLI and process-lifetime acceptance for native slots with isolated fake compilers.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!("native-slots-{name}-{}", std::process::id()));
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname='slot-process-fixture'\nversion='0.0.0'\n[workspace]\n",
        )
        .unwrap();
        std::fs::write(root.join("src/lib.rs"), "pub fn fixture() {}\n").unwrap();
        Self(root)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_justrust"));
        command
            .current_dir(&self.0)
            .env("JUSTRUST_HOME", self.0.join("home"))
            .env("JUSTRUST_TOOLCHAIN_DIST", "off")
            .env("JUSTRUST_NO_UPDATE", "1")
            .env("JUSTRUST_NO_TELEMETRY", "1")
            .env("JUSTRUST_RECORDS", "off")
            .env("JUSTRUST_REMOTE", "off")
            .env(
                "JUSTRUST_LOCAL_REASON",
                "isolated native compiler lease process acceptance",
            )
            .env("JUSTRUST_DEPCACHE", "off")
            .env_remove("JUSTRUST_REMOTE_REQUIRED")
            .env_remove("JUSTRUST_RUN_DIR")
            .env_remove("JUSTRUST_DISABLE")
            .env_remove("JUSTRUST_TOOLCHAIN")
            .env_remove("JUSTRUST_TOOLCHAIN_REQUIRED")
            .env_remove("CARGO_TARGET_DIR");
        command
    }
    fn slot(&self, args: &[&str]) -> serde_json::Value {
        let output = self
            .command()
            .args(["toolchain", "slots"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "slot {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }
    fn prefix(&self, id: &str) -> PathBuf {
        self.0.join("home/toolchains/managed/staged").join(id)
    }
    fn install_fixture(&self, id: &str) {
        let prefix = self.prefix(id);
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        let rustc = "#!/bin/sh\necho 'rustc 1.101.0-nightly (fixture 2026-10-07)'\n";
        let cargo = "#!/bin/sh\nfor arg in \"$@\"; do if [ \"$arg\" = check ] && [ -n \"$SLOT_BARRIER\" ]; then echo ready > \"$SLOT_BARRIER\"; read line || true; break; fi; done\nexit 0\n";
        for (name, body) in [("rustc", rustc), ("cargo", cargo)] {
            let path = prefix.join("bin").join(name);
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        std::fs::write(
            prefix.join("justrust-toolchain.json"),
            serde_json::to_vec(&serde_json::json!({
                "id": id, "manifest_date": "fixture", "manifest_sha256": "fixture",
                "rustc_version": "fixture", "components": [], "installed_at": 0,
                "install_secs": 0
            }))
            .unwrap(),
        )
        .unwrap();
    }
    fn pair(&self) {
        self.install_fixture("jr-aaaaaaa");
        assert_eq!(
            self.slot(&["init", "jr-aaaaaaa"])["last_good"],
            "jr-aaaaaaa"
        );
        self.install_fixture("jr-bbbbbbb");
        assert_eq!(self.slot(&["begin", "jr-bbbbbbb"])["validated"], false);
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn cli_discards_failed_slot_and_preserves_unmanaged_installation() {
    let fixture = Fixture::new("cli");
    fixture.pair();
    let legacy = fixture.0.join("home/toolchains/staged/jr-bbbbbbb");
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join("keep"), b"ordinary installation").unwrap();
    let discarded = fixture.slot(&["discard", "jr-bbbbbbb"]);
    assert_eq!(discarded["discarded"], "jr-bbbbbbb");
    assert_eq!(discarded["active"], "jr-aaaaaaa");
    assert!(!fixture.prefix("jr-bbbbbbb").exists());
    assert!(legacy.join("keep").exists());
    fixture.install_fixture("jr-ccccccc");
    assert_eq!(
        fixture.slot(&["begin", "jr-ccccccc"])["discarded"],
        serde_json::Value::Null
    );
    fixture.slot(&["validate", "jr-ccccccc"]);
    assert_eq!(fixture.slot(&["cleanup"])["status"], "cleaned");
    assert!(!fixture.prefix("jr-aaaaaaa").exists());
    assert!(fixture.prefix("jr-ccccccc").exists());
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !path.exists() {
        assert!(
            Instant::now() < deadline,
            "compiler process did not reach barrier"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn cargo_keeps_usage_lease_after_recording_parent_is_killed() {
    let fixture = Fixture::new("orphan");
    fixture.pair();
    let barrier = fixture.0.join("cargo-ready");
    let child = fixture
        .command()
        .arg("check")
        .env("JUSTRUST_TOOLCHAIN", "jr-aaaaaaa")
        .env("JUSTRUST_TOOLCHAIN_REQUIRED", "1")
        .env("SLOT_BARRIER", &barrier)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut child = Running(child);
    let input = child.0.stdin.take().unwrap();
    wait_for(&barrier);
    fixture.slot(&["validate", "jr-bbbbbbb"]);
    assert_eq!(fixture.slot(&["cleanup"])["status"], "busy");
    child.0.kill().unwrap();
    child.0.wait().unwrap();
    assert_eq!(
        fixture.slot(&["cleanup"])["status"],
        "busy",
        "orphaned Cargo must still retain the compiler"
    );
    drop(input);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let result = fixture.slot(&["cleanup"]);
        if result["status"] == "cleaned" {
            break;
        }
        assert!(Instant::now() < deadline, "orphaned Cargo did not drain");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(!fixture.prefix("jr-aaaaaaa").exists());
}
