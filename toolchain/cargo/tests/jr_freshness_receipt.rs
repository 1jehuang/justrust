//! Public native CLI acceptance: receipts describe Cargo's actual fresh inputs,
//! never authorize whole-command replay, and cannot change build results.
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

fn fixture(manifest_extra: &str, source: &str) -> TempDir {
    let temp = TempDir::new().unwrap();
    fs::create_dir(temp.path().join("src")).unwrap();
    fs::write(temp.path().join("Cargo.toml"), format!("[package]\nname=\"receipt-fixture\"\nversion=\"0.0.0\"\nedition=\"2024\"\n{manifest_extra}\n[workspace]\n")).unwrap();
    fs::write(temp.path().join("src/lib.rs"), source).unwrap();
    temp
}

fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cargo"));
    command
        .current_dir(root)
        .args(["check", "--lib", "--offline"])
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_BUILD_RUSTFLAGS")
        .env_remove("CARGO_BUILD_RUSTC_WRAPPER")
        .env_remove("CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER")
        .env_remove("CARGO_TARGET_DIR")
        .env_remove("JR_RECEIPT_TEST_VALUE")
        .env("RUSTC", "rustc")
        .env(
            "JUSTRUST_CARGO_FRESHNESS_RECEIPT",
            root.join("receipt.json"),
        )
        .env("JUSTRUST_CARGO_FRESHNESS_NONCE", "public-native-fixture-v1");
    command
}

fn run(command: &mut Command) -> Output {
    let output = command.output().unwrap();
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.is_empty(), "receipt must not add stdout");
    output
}

fn content_command(root: &Path) -> Command {
    let mut cmd = command(root);
    cmd.args([
        "-Zchecksum-freshness",
        "--config",
        "build.fingerprint=\"content\"",
    ])
    .env("__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS", "nightly")
    .env("RUSTC_BOOTSTRAP", "1");
    cmd
}

#[test]
fn content_closure_invalidates_backdated_includes_modules_and_environment() {
    let temp = fixture(
        "",
        "mod nested;\npub const TEXT: &str = include_str!(\"input.txt\");\npub const BYTES: &[u8] = include_bytes!(\"input.bin\");\npub const ENV: Option<&str> = option_env!(\"JR_RECEIPT_TEST_VALUE\");\n",
    );
    let root = temp.path();
    fs::write(root.join("src/nested.rs"), "pub const VALUE: u8 = 1;\n").unwrap();
    fs::write(root.join("src/input.txt"), "alpha").unwrap();
    fs::write(root.join("src/input.bin"), b"12345").unwrap();
    run(&mut content_command(root));
    run(&mut content_command(root));
    assert_eq!(receipt(root)["prerequisite_supported"], true);
    for (name, bytes) in [
        ("input.txt", b"bravo".as_slice()),
        ("input.bin", b"54321".as_slice()),
        ("nested.rs", b"pub const VALUE: u8 = 2;\n".as_slice()),
    ] {
        let path = root.join("src").join(name);
        let old = fs::metadata(&path).unwrap();
        assert_eq!(old.len(), bytes.len() as u64);
        fs::write(&path, bytes).unwrap();
        filetime::set_file_mtime(&path, filetime::FileTime::from_last_modification_time(&old))
            .unwrap();
        run(&mut content_command(root));
        assert_eq!(receipt(root)["units"][0]["native_fresh"], false, "{name}");
        run(&mut content_command(root));
        let fresh = receipt(root);
        assert_eq!(fresh["prerequisite_supported"], true, "{name}: {fresh:#}");
        let input = fresh["units"][0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["path"] == path.to_str().unwrap())
            .unwrap();
        assert!(input["native_dep_info_checksum"].is_string());
    }
    run(content_command(root).env("JR_RECEIPT_TEST_VALUE", "private-test-value"));
    assert_eq!(receipt(root)["units"][0]["native_fresh"], false);
    run(content_command(root).env("JR_RECEIPT_TEST_VALUE", "private-test-value"));
    let fresh = receipt(root);
    assert_eq!(fresh["prerequisite_supported"], true);
    assert!(!fresh.to_string().contains("private-test-value"));
    run(&mut content_command(root));
    assert_eq!(receipt(root)["units"][0]["native_fresh"], false);
}

fn receipt(root: &Path) -> Value {
    let value: Value =
        serde_json::from_slice(&fs::read(root.join("receipt.json")).unwrap()).unwrap();
    assert_eq!(value["schema"], 1);
    assert_eq!(value["scope"], "unit-freshness-input-prerequisite");
    assert_eq!(value["command_replay_eligible"], false);
    assert_eq!(value["trusted"], false);
    assert_eq!(value["content_snapshot_authorizes_freshness"], false);
    assert_eq!(value["native_result"], "succeeded");
    assert!(!value["replay_denials"].as_array().unwrap().is_empty());
    value
}

#[test]
fn content_freshness_receipt_checks_native_checksums_without_authorizing_replay() {
    let temp = fixture("", "pub const VALUE: u8 = 1;\n");
    let root = temp.path();
    let content_command = || {
        let mut cmd = command(root);
        cmd.args([
            "-Zchecksum-freshness",
            "--config",
            "build.fingerprint=\"content\"",
        ])
        .env("__CARGO_TEST_CHANNEL_OVERRIDE_DO_NOT_USE_THIS", "nightly")
        .env("RUSTC_BOOTSTRAP", "1");
        cmd
    };
    run(&mut content_command());
    run(&mut content_command());
    let before = receipt(root);
    assert_eq!(before["prerequisite_supported"], true, "{before:#}");
    assert_eq!(before["units"][0]["native_freshness_method"], "content");
    let path = root.join("src/lib.rs");
    let input = before["units"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == path.to_str().unwrap())
        .unwrap();
    assert!(
        input["native_dep_info_checksum"]
            .as_str()
            .unwrap()
            .starts_with("blake3=")
    );
    assert_eq!(
        input["native_dep_info_length"],
        fs::metadata(&path).unwrap().len()
    );
    let old_mtime = filetime::FileTime::from_last_modification_time(&fs::metadata(&path).unwrap());
    fs::write(&path, "pub const VALUE: u8 = 2;\n").unwrap();
    filetime::set_file_mtime(&path, old_mtime).unwrap();
    run(&mut content_command());
    assert_eq!(receipt(root)["units"][0]["native_fresh"], false);
    run(&mut content_command());
    let after = receipt(root);
    assert_eq!(after["prerequisite_supported"], true, "{after:#}");
    assert_eq!(
        after["units"][0]["compiler_read_content_bound_to_outputs"],
        false
    );
    let updated = after["units"][0]["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == path.to_str().unwrap())
        .unwrap();
    assert_ne!(
        input["native_dep_info_checksum"],
        updated["native_dep_info_checksum"]
    );
    filetime::set_file_mtime(
        &path,
        filetime::FileTime::from_unix_time(old_mtime.unix_seconds() + 60, 0),
    )
    .unwrap();
    run(&mut content_command());
    assert_eq!(receipt(root)["prerequisite_supported"], true);
}

#[test]
fn backdated_same_length_edit_is_observational_not_content_authority() {
    let temp = fixture("", "pub const VALUE: u8 = 1;\n");
    let root = temp.path();
    run(&mut command(root));
    run(&mut command(root));
    let before = receipt(root);
    assert_eq!(before["prerequisite_supported"], true);
    let path = root.join("src/lib.rs");
    let old_mtime = filetime::FileTime::from_last_modification_time(&fs::metadata(&path).unwrap());
    fs::write(&path, "pub const VALUE: u8 = 2;\n").unwrap();
    filetime::set_file_mtime(&path, old_mtime).unwrap();
    run(&mut command(root));
    let after = receipt(root);
    // Native mtime freshness is intentionally still Cargo's decision. The
    // producer must not upgrade a newly observed digest into historical proof.
    assert_eq!(after["units"][0]["native_fresh"], true, "{after:#}");
    assert_eq!(
        after["units"][0]["compiler_read_content_bound_to_outputs"],
        false
    );
    let digest = |value: &Value| {
        value["units"][0]["files"]
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == path.to_str().unwrap())
            .unwrap()["sha256"]
            .clone()
    };
    assert_ne!(digest(&before), digest(&after));
    assert!(
        after["replay_denials"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "compiler-read-content-to-output-binding-not-captured")
    );
}

#[test]
fn fresh_leaf_reports_native_depinfo_environment_outputs_and_nonce() {
    let temp = fixture(
        "",
        "mod nested;\npub const INPUT: &str = include_str!(\"input.txt\");\npub const ENV: Option<&str> = option_env!(\"JR_RECEIPT_TEST_VALUE\");\n",
    );
    let root = temp.path();
    fs::write(root.join("src/nested.rs"), "pub const VALUE: u8 = 1;\n").unwrap();
    fs::write(root.join("src/input.txt"), "first").unwrap();
    run(&mut command(root));
    assert_eq!(
        receipt(root)["prerequisite_supported"],
        false,
        "cold compile is not a fresh decision"
    );
    run(&mut command(root));
    let value = receipt(root);
    assert_eq!(value["prerequisite_supported"], true, "{value:#}");
    assert_eq!(value["invocation_nonce"], "public-native-fixture-v1");
    let unit = &value["units"][0];
    assert_eq!(unit["native_fresh"], true);
    let files = unit["files"].as_array().unwrap();
    for relative in ["src/lib.rs", "src/nested.rs", "src/input.txt", "Cargo.toml"] {
        assert!(
            files
                .iter()
                .any(|file| file["path"] == root.join(relative).to_str().unwrap()),
            "missing {relative}: {value:#}"
        );
    }
    assert!(
        files
            .iter()
            .all(|file| file["sha256"].as_str().unwrap().starts_with("sha256="))
    );
    assert!(!unit["outputs"].as_array().unwrap().is_empty());
    assert!(
        unit["environment"]
            .as_array()
            .unwrap()
            .iter()
            .any(|env| env["name"] == "JR_RECEIPT_TEST_VALUE" && env["expected_present"] == false)
    );
    run(command(root).env_remove("JUSTRUST_CARGO_FRESHNESS_NONCE"));
    let unbound = receipt(root);
    assert_eq!(unbound["prerequisite_supported"], false);
    assert!(
        unbound["unsupported_reasons"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reason| reason == "missing-invocation-nonce")
    );
    fs::remove_file(root.join("receipt.json")).unwrap();
    run(command(root).env_remove("JUSTRUST_CARGO_FRESHNESS_RECEIPT"));
    assert!(!root.join("receipt.json").exists(), "default is no receipt");
    run(command(root).env(
        "JUSTRUST_CARGO_FRESHNESS_RECEIPT",
        root.join("missing/receipt.json"),
    ));
    #[cfg(unix)]
    {
        use std::os::unix::{fs::PermissionsExt, fs::symlink};
        run(command(root).env("JUSTRUST_CARGO_FRESHNESS_RECEIPT", "/"));
        fs::write(root.join("sentinel"), "must remain unchanged").unwrap();
        symlink("sentinel", root.join("receipt.json")).unwrap();
        run(&mut command(root));
        assert_eq!(
            fs::read_to_string(root.join("sentinel")).unwrap(),
            "must remain unchanged"
        );
        assert!(!root.join("receipt.json").is_symlink());
        assert_eq!(
            fs::metadata(root.join("receipt.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn failed_builds_and_external_inputs_never_produce_supported_prerequisites() {
    let temp = fixture(
        "",
        "compile_error!(\"expected receipt fixture failure\");\n",
    );
    let output = command(temp.path()).output().unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let value: Value =
        serde_json::from_slice(&fs::read(temp.path().join("receipt.json")).unwrap()).unwrap();
    assert_eq!(value["native_result"], "failed");
    assert_eq!(value["prerequisite_supported"], false);
    assert_eq!(value["trusted"], false);
    assert_eq!(value["command_replay_eligible"], false);

    let external = TempDir::new().unwrap();
    fs::write(external.path().join("input"), "external input").unwrap();
    fs::write(
        temp.path().join("src/lib.rs"),
        format!(
            "pub const INPUT: &str = include_str!({:?});\n",
            external.path().join("input")
        ),
    )
    .unwrap();
    run(&mut command(temp.path()));
    run(&mut command(temp.path()));
    let value = receipt(temp.path());
    assert_eq!(value["prerequisite_supported"], false);
    assert!(
        value
            .to_string()
            .contains("external-source-input-unsupported")
    );
}

#[test]
fn changed_inputs_and_environment_cannot_keep_supported_receipt() {
    let temp = fixture(
        "",
        "pub const INPUT: &str = include_str!(\"input.txt\");\npub const ENV: Option<&str> = option_env!(\"JR_RECEIPT_TEST_VALUE\");\n",
    );
    let root = temp.path();
    fs::write(root.join("src/input.txt"), "first").unwrap();
    run(&mut command(root));
    run(&mut command(root));
    assert_eq!(receipt(root)["prerequisite_supported"], true);
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(root.join("src/input.txt"), "second").unwrap();
    run(&mut command(root));
    assert_eq!(receipt(root)["prerequisite_supported"], false);
    run(&mut command(root));
    assert_eq!(receipt(root)["prerequisite_supported"], true);
    run(command(root).env("JR_RECEIPT_TEST_VALUE", "receipt-private-value"));
    assert_eq!(receipt(root)["prerequisite_supported"], false);
    run(command(root).env("JR_RECEIPT_TEST_VALUE", "receipt-private-value"));
    let value = receipt(root);
    assert_eq!(value["prerequisite_supported"], true);
    assert!(
        !fs::read_to_string(root.join("receipt.json"))
            .unwrap()
            .contains("receipt-private-value"),
        "environment values must be digested"
    );
    let output = value["units"][0]["outputs"][0]["path"].as_str().unwrap();
    fs::remove_file(output).unwrap();
    run(command(root).env("JR_RECEIPT_TEST_VALUE", "receipt-private-value"));
    assert_eq!(receipt(root)["prerequisite_supported"], false);
}

#[test]
fn scripts_proc_macros_dependencies_and_flags_are_explicitly_unsupported() {
    let scripted = fixture("", "pub const VALUE: u8 = 1;\n");
    fs::write(
        scripted.path().join("build.rs"),
        "fn main() { println!(\"cargo::rerun-if-changed=src/lib.rs\"); }\n",
    )
    .unwrap();
    run(&mut command(scripted.path()));
    run(&mut command(scripted.path()));
    let value = receipt(scripted.path());
    assert_eq!(value["prerequisite_supported"], false);
    assert!(value.to_string().contains("build-script-unsupported"));

    let macros = fixture("[lib]\nproc-macro=true", "extern crate proc_macro;\n");
    run(&mut command(macros.path()));
    run(&mut command(macros.path()));
    let value = receipt(macros.path());
    assert_eq!(value["prerequisite_supported"], false);
    assert!(value.to_string().contains("proc-macro-unsupported"));

    let dependent = fixture(
        "[dependencies]\ndep={path=\"dep\"}",
        "pub use dep::VALUE;\n",
    );
    fs::create_dir_all(dependent.path().join("dep/src")).unwrap();
    fs::write(
        dependent.path().join("dep/Cargo.toml"),
        "[package]\nname=\"dep\"\nversion=\"0.0.0\"\nedition=\"2024\"\n",
    )
    .unwrap();
    fs::write(
        dependent.path().join("dep/src/lib.rs"),
        "pub const VALUE: u8 = 1;\n",
    )
    .unwrap();
    run(&mut command(dependent.path()));
    run(&mut command(dependent.path()));
    let value = receipt(dependent.path());
    assert_eq!(value["prerequisite_supported"], false);
    assert!(value.to_string().contains("unsupported-unit-closure"));
    assert!(value.to_string().contains("dependency-closure-unsupported"));

    let flags = fixture("", "pub const VALUE: u8 = 1;\n");
    for _ in 0..2 {
        run(command(flags.path()).env("RUSTFLAGS", "-Cdebuginfo=0"));
    }
    let value = receipt(flags.path());
    assert_eq!(value["prerequisite_supported"], false);
    assert!(
        value
            .to_string()
            .contains("extra-compiler-flags-unsupported")
    );
}
