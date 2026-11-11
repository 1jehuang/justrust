//! Real CLI routing guards for Windows hosted selection. No credentials,
//! network, Cargo installation or native Windows host is used by these tests.
#![cfg(unix)]
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "justrust-windows-cli-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("home")).unwrap();
        Self(root)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_justrust"));
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", self.0.join("home"))
            .env("JUSTRUST_HOME", self.0.join("state"))
            .env("JCODE_HOME", self.0.join("jcode"))
            .env("XDG_CONFIG_HOME", self.0.join("config"))
            .env("JUSTRUST_NO_UPDATE", "1")
            .env("JUSTRUST_NO_TELEMETRY", "1")
            .env("JUSTRUST_REQUIRE_LOCAL_REASON", "0")
            .current_dir(&self.0);
        command
    }
    fn assert_no_local_build(&self) {
        assert!(!self.0.join("target").exists());
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn explicit_windows_never_falls_back_to_local_when_hosted_is_unconfigured() {
    let fixture = Fixture::new();
    for args in [
        vec!["check", "--host-platform", "windows"],
        vec!["test", "--host-platform=windows"],
        vec!["build", "--host-platform=windows", "--auto"],
    ] {
        let output = fixture.command().args(&args).output().unwrap();
        assert!(!output.status.success(), "{args:?}: {output:?}");
        assert!(
            stderr(&output).contains("no remote machine configured"),
            "{args:?}: {}",
            stderr(&output)
        );
        fixture.assert_no_local_build();
    }
}

#[test]
fn saved_windows_selection_and_environment_override_use_the_same_fail_closed_route() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.0.join("state/remote")).unwrap();
    std::fs::write(
        fixture.0.join("state/remote/hosted-platform.json"),
        "\"windows\"",
    )
    .unwrap();
    let saved = fixture.command().arg("test").output().unwrap();
    assert!(!saved.status.success());
    assert!(
        stderr(&saved).contains("no remote machine configured"),
        "{}",
        stderr(&saved)
    );
    std::fs::remove_file(fixture.0.join("state/remote/hosted-platform.json")).unwrap();
    let override_result = fixture
        .command()
        .env("JUSTRUST_HOST_PLATFORM", "windows")
        .arg("check")
        .output()
        .unwrap();
    assert!(!override_result.status.success());
    assert!(
        stderr(&override_result).contains("no remote machine configured"),
        "{}",
        stderr(&override_result)
    );
    fixture.assert_no_local_build();
}

#[test]
fn windows_local_mode_cannot_execute_on_a_unix_client() {
    let fixture = Fixture::new();
    let output = fixture
        .command()
        .args(["check", "--host-platform=windows", "--local"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("cannot run locally"),
        "{}",
        stderr(&output)
    );
    fixture.assert_no_local_build();
}

#[test]
fn malformed_saved_platform_and_unknown_setup_platform_fail_before_cargo() {
    let fixture = Fixture::new();
    std::fs::create_dir_all(fixture.0.join("state/remote")).unwrap();
    std::fs::write(fixture.0.join("state/remote/hosted-platform.json"), "null").unwrap();
    let malformed = fixture.command().arg("check").output().unwrap();
    assert!(!malformed.status.success());
    assert!(
        stderr(&malformed).contains("invalid hosted platform selection"),
        "{}",
        stderr(&malformed)
    );
    for args in [
        vec!["check", "--host-platform=win"],
        vec!["remote", "use", "hosted", "--platform=macos"],
        vec![
            "remote",
            "use",
            "hosted",
            "--platform=windows",
            "--target=x86_64-unknown-linux-gnu",
        ],
    ] {
        let output = fixture.command().args(&args).output().unwrap();
        assert!(!output.status.success(), "{args:?}: {output:?}");
        assert!(
            stderr(&output).contains("platform"),
            "{args:?}: {}",
            stderr(&output)
        );
        fixture.assert_no_local_build();
    }
}

#[test]
fn setup_persists_host_platform_and_routes_before_linux_test_reuse() {
    let fixture = Fixture::new();
    let setup = fixture
        .command()
        .args(["setup", "set", "host-platform", "windows"])
        .output()
        .unwrap();
    assert!(setup.status.success(), "{}", stderr(&setup));
    let run = fixture.command().arg("test").output().unwrap();
    assert!(!run.status.success());
    assert!(
        stderr(&run).contains("no remote machine configured"),
        "{}",
        stderr(&run)
    );
    fixture.assert_no_local_build();
}
