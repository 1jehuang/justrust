//! Unix subprocess contract for the explicit native dispatcher, using no real engine.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
    engine: PathBuf,
}

impl Fixture {
    fn new(script: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "justrust-native-cli-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        // create_dir refuses a preexisting fixture instead of reusing stale state.
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let home = root.join("empty home");
        let cwd = root.join("working directory with spaces");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&cwd).unwrap();
        let engine = root.join("fixture engine with spaces");
        fs::write(&engine, format!("#!/bin/sh\n{script}\n")).unwrap();
        fs::set_permissions(&engine, fs::Permissions::from_mode(0o700)).unwrap();
        Self {
            root,
            home,
            cwd,
            engine,
        }
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_justrust"));
        // Do not inherit accounts, daemon, engine, or toolchain configuration from
        // the developer's machine. Each process gets a genuinely empty home.
        command
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("HOME", &self.home)
            .env("JUSTRUST_HOME", self.root.join("justrust state"))
            .env("XDG_CONFIG_HOME", self.root.join("absent config"))
            .env("XDG_CACHE_HOME", self.root.join("absent cache"))
            .env("JUSTRUST_NATIVE_ENGINE", &self.engine)
            .current_dir(&self.cwd)
            .arg("native");
        command
    }

    fn assert_no_configuration_created(&self) {
        assert_eq!(fs::read_dir(&self.home).unwrap().count(), 0);
        assert!(!self.root.join("absent config").exists());
        assert!(!self.root.join("absent cache").exists());
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn native_preserves_exact_argv_separator_spaces_and_empty_argument() {
    let fixture = Fixture::new("printf '%s\\000' \"$@\"");
    let arguments = [
        "test",
        "--lib",
        "--",
        "filter with spaces",
        "",
        "--nocapture",
    ];
    let output = fixture.command().args(arguments).output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let expected: Vec<u8> = std::iter::once("cargo")
        .chain(arguments)
        .flat_map(|argument| argument.bytes().chain(std::iter::once(0)))
        .collect();
    assert_eq!(output.stdout, expected);
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
    fixture.assert_no_configuration_created();
}

#[test]
fn native_forwards_each_supported_subcommand_without_rewriting() {
    let fixture = Fixture::new("printf '%s\\000' \"$@\"");
    for subcommand in [
        "build",
        "test",
        "check",
        "run",
        "status",
        "stop",
        "toolchain",
    ] {
        let output = fixture.command().arg(subcommand).output().unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        assert_eq!(output.stdout, format!("cargo\0{subcommand}\0").as_bytes());
        assert!(output.stderr.is_empty(), "{}", stderr(&output));
    }
}

#[test]
fn native_inherits_callers_environment_and_working_directory() {
    let fixture = Fixture::new("printf '%s\\000' \"$NATIVE_CLI_SENTINEL\"; pwd -P");
    let output = fixture
        .command()
        .env("NATIVE_CLI_SENTINEL", "inherited value with spaces")
        .arg("check")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let expected = format!(
        "inherited value with spaces\0{}\n",
        fixture.cwd.canonicalize().unwrap().display()
    );
    assert_eq!(output.stdout, expected.as_bytes());
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
}

#[test]
fn native_preserves_stdout_stderr_and_exit_101_without_retry() {
    let fixture =
        Fixture::new("printf 'engine stdout\\n'; printf 'engine diagnostic\\n' >&2; exit 101");
    let output = fixture.command().arg("test").output().unwrap();
    assert_eq!(output.status.code(), Some(101));
    assert_eq!(output.stdout, b"engine stdout\n");
    assert_eq!(output.stderr, b"engine diagnostic\n");
    fixture.assert_no_configuration_created();
}

#[test]
fn native_preserves_sigterm_instead_of_converting_it_to_an_exit_code() {
    let fixture = Fixture::new("kill -TERM \"$$\"");
    let output = fixture.command().arg("run").output().unwrap();
    assert_eq!(output.status.signal(), Some(libc::SIGTERM));
    assert_eq!(output.status.code(), None);
    assert!(output.stdout.is_empty());
    assert!(output.stderr.is_empty(), "{}", stderr(&output));
}

#[test]
fn native_missing_engine_has_actionable_error_and_no_cargo_retry() {
    let fixture = Fixture::new("exit 99");
    let missing = fixture.root.join("missing owned engine");
    let output = fixture
        .command()
        .env("JUSTRUST_NATIVE_ENGINE", &missing)
        .arg("build")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    let error = stderr(&output);
    assert!(error.contains("native engine not available"), "{error}");
    assert!(error.contains("build-jr.sh"), "{error}");
    assert!(error.contains("JUSTRUST_NATIVE_ENGINE"), "{error}");
    assert!(error.contains("still use Cargo"), "{error}");
    assert!(error.contains(&missing.display().to_string()), "{error}");
    fixture.assert_no_configuration_created();
}

#[test]
fn native_refuses_relative_engine_override() {
    let fixture = Fixture::new("exit 99");
    let output = fixture
        .command()
        .env("JUSTRUST_NATIVE_ENGINE", "relative-engine")
        .arg("check")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("must be an absolute path"));
}

#[test]
fn native_refuses_self_override_without_recursive_execution() {
    let fixture = Fixture::new("exit 99");
    let binary = PathBuf::from(env!("CARGO_BIN_EXE_justrust"))
        .canonicalize()
        .unwrap();
    let output = fixture
        .command()
        .env("JUSTRUST_NATIVE_ENGINE", binary)
        .arg("check")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("refusing recursive execution"));
}

#[test]
fn native_unsupported_command_does_not_execute_the_fixture() {
    let fixture = Fixture::new("printf 'executed' > \"$NATIVE_CLI_MARKER\"; exit 99");
    let marker = fixture.root.join("unexpected execution");
    let output = fixture
        .command()
        .env("NATIVE_CLI_MARKER", &marker)
        .arg("clippy")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert!(stderr(&output).contains("unsupported native command"));
    assert!(!marker.exists());
    fixture.assert_no_configuration_created();
}

#[test]
fn native_help_needs_neither_engine_nor_default_configuration() {
    let fixture = Fixture::new("exit 99");
    for arguments in [vec!["--help"], vec!["-h"], vec![]] {
        let output = fixture
            .command()
            .env("JUSTRUST_NATIVE_ENGINE", fixture.root.join("not installed"))
            .args(arguments)
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", stderr(&output));
        let help = std::str::from_utf8(&output.stdout).unwrap();
        assert!(help.contains("justrust native <build|test|check|run|status|stop|toolchain>"));
        assert!(help.contains("JUSTRUST_NATIVE_ENGINE"));
        assert!(help.contains("jr-native"));
        assert!(output.stderr.is_empty(), "{}", stderr(&output));
        fixture.assert_no_configuration_created();
        assert!(!fixture.root.join("justrust state").exists());
    }
}

#[test]
fn native_default_engine_home_is_canonical_and_owner_only() {
    let fixture = Fixture::new("printf '%s' \"$JR_ENGINE_HOME\"");
    let output = fixture.command().arg("status").output().unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    let expected = fixture.root.join("justrust state").join("native");
    assert_eq!(output.stdout, expected.as_os_str().as_encoded_bytes());
    assert_eq!(expected.canonicalize().unwrap(), expected);
    assert_eq!(
        fs::metadata(&expected).unwrap().permissions().mode() & 0o777,
        0o700
    );
    fixture.assert_no_configuration_created();
}

#[test]
fn native_explicit_unsafe_engine_home_is_forwarded_without_chmod() {
    let fixture = Fixture::new("printf '%s' \"$JR_ENGINE_HOME\"");
    let explicit = fixture.root.join("explicit unsafe engine home");
    fs::create_dir(&explicit).unwrap();
    fs::set_permissions(&explicit, fs::Permissions::from_mode(0o755)).unwrap();
    let output = fixture
        .command()
        .env("JR_ENGINE_HOME", &explicit)
        .arg("status")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, explicit.as_os_str().as_encoded_bytes());
    assert_eq!(
        fs::metadata(&explicit).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert!(!fixture.root.join("justrust state").exists());
    fixture.assert_no_configuration_created();
}

#[test]
fn native_explicit_engine_home_is_not_created_or_normalized_by_client() {
    let fixture = Fixture::new("printf '%s' \"$JR_ENGINE_HOME\"");
    // The engine, not the client, owns validation of an explicit override.
    let explicit = "uncreated relative engine home";
    let output = fixture
        .command()
        .env("JR_ENGINE_HOME", explicit)
        .arg("status")
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(output.stdout, explicit.as_bytes());
    assert!(!fixture.cwd.join(explicit).exists());
    assert!(!fixture.root.join("justrust state").exists());
    fixture.assert_no_configuration_created();
}
