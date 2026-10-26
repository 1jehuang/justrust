//! CI must never silently replace a requested hosted build with a local build.
use std::process::Command;

#[test]
fn strict_remote_rejects_local_and_unsupported_builds_before_cargo() {
    let root = std::env::temp_dir().join(format!("jr-required-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for args in [
        vec!["build", "--local", "--release"],
        vec!["build", "--remote"],
        vec!["test", "--local", "--no-run"],
        vec![
            "test",
            "--remote",
            "--no-run",
            "--target=wasm32-unknown-unknown",
        ],
        vec![
            "test",
            "--remote",
            "--no-run",
            "--target=aarch64-unknown-linux-musl",
        ],
        vec![
            "build",
            "--remote",
            "--release",
            "--target=wasm32-unknown-unknown",
        ],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_justrust"))
            .args(&args)
            .current_dir(&root)
            .env("JUSTRUST_HOME", root.join("home"))
            // These fixtures test routing guards, not detached background work.
            .env("JUSTRUST_NO_TELEMETRY", "1")
            .env("JUSTRUST_NO_UPDATE", "1")
            .env("JUSTRUST_REMOTE_REQUIRED", "1")
            .env_remove("JUSTRUST_ON_REMOTE_AGENT")
            .env_remove("JUSTRUST_RUN_ID_OVERRIDE")
            .env_remove("JCODE_API_KEY")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr)
                .contains("remote required, refusing local build"),
            "{args:?}: {output:?}"
        );
        assert!(!root.join("target").exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn strict_toolchain_rejects_system_and_uninstalled_pins_before_cargo() {
    let root = std::env::temp_dir().join(format!("jr-required-toolchain-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"strict-toolchain-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n[lib]\npath = \"lib.rs\"\n").unwrap();
    std::fs::write(root.join("lib.rs"), "pub fn fixture() {}\n").unwrap();
    for (pin, disable) in [
        ("system", "0"),
        ("0.0.0", "0"),
        ("system", "1"),
        ("0.0.0", "1"),
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_justrust"))
            .args(["build", "--local", "--release"])
            .current_dir(&root)
            .env("JUSTRUST_HOME", root.join("home"))
            // These fixtures test routing guards, not detached background work.
            .env("JUSTRUST_NO_TELEMETRY", "1")
            .env("JUSTRUST_NO_UPDATE", "1")
            .env("JUSTRUST_TOOLCHAIN", pin)
            .env("JUSTRUST_TOOLCHAIN_REQUIRED", "1")
            .env("JUSTRUST_TOOLCHAIN_AUTO_INSTALL", "0")
            .env_remove("JUSTRUST_REMOTE_REQUIRED")
            .env_remove("JUSTRUST_RUN_DIR")
            .env("JUSTRUST_DISABLE", disable)
            .env_remove("JUSTRUST_ON_REMOTE_AGENT")
            .env_remove("JUSTRUST_RUN_ID_OVERRIDE")
            .env_remove("JCODE_API_KEY")
            .output()
            .unwrap();
        assert!(!output.status.success(), "{pin}: {output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains("toolchain required"),
            "{pin}: {output:?}"
        );
        assert!(!root.join("target").exists());
    }
    std::fs::remove_dir_all(root).unwrap();
}
