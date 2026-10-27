//! Cargo project support: manifests, workspaces, path dependencies,
//! features, targets, `#[test]` harness and integration tests, driven
//! through `jr-rust check|build|test|run` and compared with stock Cargo.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "support/stock_rustc.rs"]
mod stock_rustc;

fn stock_command() -> Command {
    let rustc = stock_rustc::stock_rustc();
    let rustdoc = Path::new(&rustc).with_file_name("rustdoc");
    let mut command = Command::new("cargo");
    command
        .env("RUSTC", rustc)
        .env("RUSTDOC", rustdoc)
        .env_remove("RUSTC_WRAPPER")
        .env_remove("RUSTC_WORKSPACE_WRAPPER")
        .env_remove("RUSTC_BOOTSTRAP");
    command
}

const CRATES_IO_CACHE_ID: &str = "index.crates.io-1949cf8c6b5b557f";

#[test]
fn sibling_path_dependencies_are_captured_for_cli_check_build_run() {
    let base = target_dir("captured-siblings-cli");
    let root = base.join("app");
    let dep = base.join("dep");
    let leaf = base.join("leaf");
    for directory in [&root, &dep, &leaf] {
        std::fs::create_dir_all(directory.join("src")).unwrap();
    }
    std::fs::create_dir_all(base.join("home")).unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname='app'\nversion='0.1.0'\nedition='2024'\n[dependencies]\ndep={path='../dep'}\n").unwrap();
    std::fs::write(dep.join("Cargo.toml"), "[package]\nname='dep'\nversion='0.1.0'\nedition='2024'\n[dependencies]\nleaf={path='../leaf'}\n").unwrap();
    std::fs::write(
        leaf.join("Cargo.toml"),
        "[package]\nname='leaf'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() { assert_eq!(dep::value(), 42); }",
    )
    .unwrap();
    std::fs::write(
        dep.join("src/lib.rs"),
        "pub fn value() -> i32 { leaf::value() }",
    )
    .unwrap();
    std::fs::write(
        leaf.join("src/lib.rs"),
        "mod child; pub fn value() -> i32 { child::value() }",
    )
    .unwrap();
    let child = leaf.join("src/child.rs");
    std::fs::write(&child, "pub fn value() -> i32 { 42 }").unwrap();
    // An unrelated sibling must neither be scanned nor satisfy an escaped read.
    std::os::unix::fs::symlink("missing", base.join("unowned")).unwrap();
    let stock = stock_command()
        .args(["run", "--quiet"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", base.join("stock"))
        .output()
        .unwrap();
    assert!(stock.status.success(), "{stock:?}");
    let invoke = |command: &str, workers: &str| {
        Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .args([command, "-j", workers])
            .current_dir(&root)
            .env("CARGO_HOME", base.join("home"))
            .env("CARGO_TARGET_DIR", base.join("native"))
            .output()
            .unwrap()
    };
    for workers in ["1", "8"] {
        for command in ["check", "build", "run"] {
            let output = invoke(command, workers);
            assert!(output.status.success(), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("Input capture: immutable native project namespace")
            );
            assert_eq!(output.stdout, stock.stdout);
        }
    }
    std::fs::remove_file(&child).unwrap();
    let missing = invoke("build", "8");
    assert!(!missing.status.success(), "{missing:?}");
    std::os::unix::fs::symlink("missing", &child).unwrap();
    let symlink = invoke("check", "1");
    assert!(!symlink.status.success());
    assert!(String::from_utf8_lossy(&symlink.stderr).contains("does not follow symlinks"));
    std::fs::remove_file(&child).unwrap();
    std::fs::write(
        &child,
        "pub fn value() -> i32 { 42 } fn unused() -> i32 { true }",
    )
    .unwrap();
    for workers in ["1", "8"] {
        let invalid = invoke("check", workers);
        assert!(!invalid.status.success(), "{invalid:?}");
    }
    std::fs::remove_dir_all(base).unwrap();
}

#[test]
fn ordinary_cli_captures_one_namespace_for_check_build_and_run() {
    let root = target_dir("captured-cli");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("home")).unwrap();
    std::fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='captured_cli'\nversion='0.1.0'\nedition='2024'\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "mod child; fn main() { assert_eq!(child::value(), 42); }",
    )
    .unwrap();
    std::fs::write(root.join("src/child.rs"), "pub fn value() -> i32 { 42 }").unwrap();
    let stock = stock_command()
        .arg("run")
        .arg("--quiet")
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("stock"))
        .output()
        .unwrap();
    assert!(stock.status.success(), "{stock:?}");
    let invoke = |command: &str, workers: &str| {
        Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .args([command, "-j", workers])
            .current_dir(&root)
            .env("CARGO_HOME", root.join("home"))
            .env("CARGO_TARGET_DIR", root.join("native"))
            .output()
            .unwrap()
    };
    for workers in ["1", "8"] {
        for command in ["check", "build", "run"] {
            let output = invoke(command, workers);
            assert!(output.status.success(), "{output:?}");
            assert!(
                String::from_utf8_lossy(&output.stderr)
                    .contains("Input capture: immutable native project namespace")
            );
            assert_eq!(output.stdout, stock.stdout);
        }
    }
    std::fs::remove_file(root.join("src/child.rs")).unwrap();
    let missing = invoke("build", "8");
    assert!(!missing.status.success());
    assert!(!String::from_utf8_lossy(&missing.stderr).contains("Finished"));
    std::os::unix::fs::symlink("missing.rs", root.join("src/child.rs")).unwrap();
    let unsupported = invoke("check", "1");
    assert!(!unsupported.status.success());
    assert!(String::from_utf8_lossy(&unsupported.stderr).contains("does not follow symlinks"));
    std::fs::remove_dir_all(root).unwrap();
}

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/projects")
        .join(name)
}

fn target_dir(name: &str) -> PathBuf {
    std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-projects-{}-{name}", std::process::id()))
}

fn jr(project: &str, args: &[&str]) -> Output {
    jr_with(project, args, [])
}

fn jr_with(
    project: &str,
    args: &[&str],
    env: impl IntoIterator<Item = (&'static str, PathBuf)>,
) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args(args)
        .current_dir(fixture(project))
        .env("CARGO_TARGET_DIR", target_dir(project))
        .envs(env)
        .output()
        .unwrap()
}

fn stock(project: &str, args: &[&str]) -> Option<Output> {
    stock_command()
        .args(args)
        .current_dir(fixture(project))
        .env("CARGO_TARGET_DIR", target_dir(&format!("{project}-stock")))
        .output()
        .ok()
}

fn seed_registry_cache(crate_stem: &str) -> PathBuf {
    let scratch = target_dir(&format!(
        "cache-{crate_stem}-{}",
        std::thread::current().name().unwrap_or("main")
    ));
    let cargo_home = scratch.join("cargo-home");
    let src_root = cargo_home.join("registry/src").join(CRATES_IO_CACHE_ID);
    let src_dir = src_root.join(crate_stem);
    let cache_dir = cargo_home.join("registry/cache").join(CRATES_IO_CACHE_ID);
    std::fs::create_dir_all(&src_root).unwrap();
    std::fs::create_dir_all(&cache_dir).unwrap();
    let archive = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/registry-cache")
        .join(format!("{crate_stem}.crate"));
    std::fs::copy(&archive, cache_dir.join(format!("{crate_stem}.crate"))).unwrap();
    let status = Command::new("tar")
        .args(["-xzf"])
        .arg(&archive)
        .args(["-C"])
        .arg(&src_root)
        .status()
        .unwrap();
    assert!(status.success(), "failed to unpack {}", archive.display());
    assert!(
        src_dir.join("Cargo.toml").is_file(),
        "missing cached source manifest at {}",
        src_dir.display()
    );
    cargo_home
}

/// libtest prints per-test durations in some configurations.
fn normalize(stdout: &[u8]) -> String {
    let text = String::from_utf8_lossy(stdout);
    text.lines()
        .map(|line| match line.find(" <") {
            Some(at) if line.ends_with("s>") => &line[..at],
            _ => line,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn workspace_check_build_run_and_test_match_stock_cargo() {
    let check = jr("workspace", &["check"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let run = jr("workspace", &["run", "--bin", "app"]);
    assert!(
        run.status.success(),
        "{}",
        String::from_utf8_lossy(&run.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&run.stdout),
        "total area 14\nfancy 42\ndouble 10\n"
    );
    let tool = jr("workspace", &["run", "-p", "app", "--bin", "tool"]);
    assert_eq!(String::from_utf8_lossy(&tool.stdout), "tool 16\n");
    let ambiguous = jr("workspace", &["run", "-p", "app"]);
    assert_eq!(ambiguous.status.code(), Some(101));
    assert!(
        String::from_utf8_lossy(&ambiguous.stderr).contains("could not determine which binary")
    );
    let test = jr("workspace", &["test", "--tests"]);
    assert!(
        test.status.success(),
        "{}",
        String::from_utf8_lossy(&test.stderr)
    );
    if let Some(expected) = stock("workspace", &["test", "--tests"]).filter(|o| o.status.success())
    {
        // Stock libtest reports parallel tests in completion order.
        let sorted = |text: String| {
            let mut lines: Vec<String> = text.lines().map(String::from).collect();
            lines.sort();
            lines
        };
        assert_eq!(
            sorted(normalize(&test.stdout)),
            sorted(normalize(&expected.stdout))
        );
    }
    let build = jr("workspace", &["build", "--release"]);
    assert!(build.status.success());
}

#[test]
fn failing_tests_report_like_libtest_and_exit_101() {
    let test = jr("failing", &["test", "--lib"]);
    assert_eq!(test.status.code(), Some(101));
    let stdout = String::from_utf8_lossy(&test.stdout);
    for line in [
        "running 4 tests",
        "test tests::halves ... ok",
        "test tests::skipped ... ignored",
        "test tests::wrong ... FAILED",
        "test tests::zero - should panic ... ok",
        "---- tests::wrong stdout ----",
        "captured output",
        "test result: FAILED. 2 passed; 1 failed; 1 ignored; 0 measured; 0 filtered out",
    ] {
        assert!(stdout.contains(line), "missing {line:?} in\n{stdout}");
    }
    assert!(
        String::from_utf8_lossy(&test.stderr).contains("error: test failed, to rerun pass `--lib`")
    );
    // Libtest filters are applied by the native harness.
    let filtered = jr("failing", &["test", "--lib", "halves"]);
    assert!(filtered.status.success());
    assert!(
        String::from_utf8_lossy(&filtered.stdout)
            .contains("test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 3 filtered out")
    );
    let ignored = jr("failing", &["test", "--lib", "--", "--ignored"]);
    assert!(String::from_utf8_lossy(&ignored.stdout).contains("test tests::skipped ... ok"));
    // A plain build of the library ignores #[test] items entirely.
    assert!(jr("failing", &["check"]).status.success());
}

#[test]
fn doctest_selection_is_explicit_and_never_reports_invented_success() {
    let project = target_dir("doctest-selection");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::create_dir_all(project.join("tests")).unwrap();
    let manifest = "[package]\nname = \"doc_selection\"\nversion = \"0.1.0\"\nedition = \"2024\"\n";
    std::fs::write(project.join("Cargo.toml"), manifest).unwrap();
    // The doctest intentionally fails, while ordinary test targets pass.
    // This makes accidental doctest execution observable in the stock oracle.
    std::fs::write(
        project.join("src/lib.rs"),
        "/// ```\n/// assert!(false);\n/// ```\npub fn value() -> i32 { 7 }\n#[test]\nfn unit_passes() { assert_eq!(value(), 7); }\n",
    )
    .unwrap();
    std::fs::write(project.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(
        project.join("tests/smoke.rs"),
        "#[test]\nfn integration_passes() { assert_eq!(doc_selection::value(), 7); }\n",
    )
    .unwrap();
    let invoke = |native: bool, args: &[&str]| {
        let mut command = if native {
            Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        } else {
            stock_command()
        };
        command
            .args(args)
            .current_dir(&project)
            .env(
                "CARGO_TARGET_DIR",
                project.join(if native { "native" } else { "stock" }),
            )
            .output()
            .unwrap()
    };
    for args in [
        vec!["test", "--lib"],
        vec!["test", "--test", "smoke"],
        vec!["test", "--tests"],
        vec!["test", "--bin", "doc_selection"],
        vec!["test", "--bins"],
        vec!["test", "--all-targets"],
        vec!["test", "--all-targets", "--lib"],
        vec!["test", "--no-run"],
    ] {
        let expected = invoke(false, &args);
        assert!(
            expected.status.success(),
            "stock {args:?}: {}",
            String::from_utf8_lossy(&expected.stderr)
        );
        assert!(!String::from_utf8_lossy(&expected.stderr).contains("Doc-tests"));
        for workers in ["1", "8"] {
            let mut native_args = args.clone();
            native_args.extend(["-j", workers]);
            let actual = invoke(true, &native_args);
            assert!(
                actual.status.success(),
                "native {native_args:?}: {}",
                String::from_utf8_lossy(&actual.stderr)
            );
            assert!(!String::from_utf8_lossy(&actual.stderr).contains("Doc-tests"));
            assert_eq!(
                normalize(&actual.stdout).contains("unit_passes"),
                normalize(&expected.stdout).contains("unit_passes")
            );
            assert_eq!(
                normalize(&actual.stdout).contains("integration_passes"),
                normalize(&expected.stdout).contains("integration_passes")
            );
        }
    }
    let expected = invoke(false, &["test"]);
    assert_eq!(expected.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&expected.stderr).contains("Doc-tests"));
    for args in [
        vec!["test"],
        vec!["test", "unit_passes"],
        vec!["test", "--workspace"],
        vec!["test", "-p", "doc_selection"],
    ] {
        for workers in ["1", "8"] {
            let mut args = args.clone();
            args.extend(["-j", workers]);
            let actual = invoke(true, &args);
            assert_eq!(actual.status.code(), Some(101));
            let stderr = String::from_utf8_lossy(&actual.stderr);
            assert!(
                stderr.contains("Unsupported: doc-tests"),
                "{args:?}: {stderr}"
            );
            assert!(!stderr.contains("Finished"), "{stderr}");
            assert!(
                actual.stdout.is_empty(),
                "must not report successful test results"
            );
        }
    }
    // A real manifest opt-out requests no doctests, even when documentation
    // contains a failing Rust example. Ordinary unit/integration tests still run.
    std::fs::write(
        project.join("Cargo.toml"),
        format!("{manifest}[lib]\ndoctest = false\n"),
    )
    .unwrap();
    let expected = invoke(false, &["test"]);
    assert!(
        expected.status.success(),
        "{}",
        String::from_utf8_lossy(&expected.stderr)
    );
    assert!(!String::from_utf8_lossy(&expected.stderr).contains("Doc-tests"));
    for workers in ["1", "8"] {
        let actual = invoke(true, &["test", "-j", workers]);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert!(!String::from_utf8_lossy(&actual.stderr).contains("Doc-tests"));
        assert!(String::from_utf8_lossy(&actual.stdout).contains("unit_passes"));
        assert!(String::from_utf8_lossy(&actual.stdout).contains("integration_passes"));
    }
    for value in ["\"false\"", "0", "[]"] {
        std::fs::write(
            project.join("Cargo.toml"),
            format!("{manifest}[lib]\ndoctest = {value}\n"),
        )
        .unwrap();
        assert!(!invoke(false, &["test", "--lib"]).status.success());
        let actual = invoke(true, &["test", "--lib"]);
        assert_eq!(actual.status.code(), Some(101));
        assert!(
            String::from_utf8_lossy(&actual.stderr).contains("lib.doctest: expected a boolean")
        );
        assert!(actual.stdout.is_empty());
    }
    // Disabling unit tests does not disable doctests, and cannot hide the
    // unsupported request merely because there are no test executables.
    let no_units = manifest.replace(
        "edition = \"2024\"\n",
        "edition = \"2024\"\nautobins = false\nautotests = false\n[lib]\ntest = false\n",
    );
    std::fs::write(project.join("Cargo.toml"), &no_units).unwrap();
    let expected = invoke(false, &["test"]);
    assert_eq!(expected.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&expected.stderr).contains("Doc-tests"));
    for workers in ["1", "8"] {
        let actual = invoke(true, &["test", "-j", workers]);
        assert_eq!(actual.status.code(), Some(101));
        assert!(String::from_utf8_lossy(&actual.stderr).contains("Unsupported: doc-tests"));
        assert!(actual.stdout.is_empty());
    }
    // Even an apparently empty source cannot justify claiming that rustdoc
    // discovered zero tests when native doctest discovery is unavailable.
    std::fs::write(project.join("src/lib.rs"), "pub fn value() -> i32 { 7 }\n").unwrap();
    let actual = invoke(true, &["test"]);
    assert_eq!(actual.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&actual.stderr).contains("Unsupported: doc-tests"));
    assert!(actual.stdout.is_empty());
}

#[test]
fn locked_registry_earlier_edition_is_explicitly_unsupported() {
    // Keep the authenticated external package unchanged. Positive registry
    // execution remains blocked until edition 2021 semantics are implemented.
    let cargo_home = seed_registry_cache("utf8-width-0.1.8");
    for workers in ["1", "8"] {
        for command in ["check", "build", "run"] {
            let output = jr_with(
                "registry",
                &[command, "-j", workers],
                [("CARGO_HOME", cargo_home.clone())],
            );
            assert_eq!(output.status.code(), Some(101));
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                stderr.contains("Rust edition 2021 is unsupported"),
                "{stderr}"
            );
            assert!(stderr.contains("utf8-width-0.1.8"), "{stderr}");
            assert!(!stderr.contains("Finished"), "{stderr}");
        }
    }
}

#[test]
fn locked_registry_dependency_reports_missing_cache_explicitly() {
    let scratch = target_dir("registry-missing-cache");
    let project = scratch.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        std::fs::read_to_string(fixture("registry").join("Cargo.toml")).unwrap(),
    )
    .unwrap();
    std::fs::write(
        project.join("Cargo.lock"),
        std::fs::read_to_string(fixture("registry").join("Cargo.lock")).unwrap(),
    )
    .unwrap();
    std::fs::write(
        project.join("src/main.rs"),
        std::fs::read_to_string(fixture("registry").join("src/main.rs")).unwrap(),
    )
    .unwrap();
    let cargo_home = scratch.join("empty-cargo-home");
    std::fs::create_dir_all(cargo_home.join("registry/src")).unwrap();
    std::fs::create_dir_all(cargo_home.join("registry/cache")).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args(["check"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .env("CARGO_HOME", &cargo_home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("locked registry package `utf8-width` 0.1.8"),
        "{stderr}"
    );
    assert!(
        stderr.contains("not cached") || stderr.contains("local Cargo cache"),
        "{stderr}"
    );
}

#[test]
fn inactive_optional_registry_source_is_not_required_but_activation_checks_it() {
    for damaged in [false, true] {
        let scratch = target_dir(if damaged {
            "optional-damaged"
        } else {
            "optional-absent"
        });
        let project = scratch.join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        let manifest = std::fs::read_to_string(fixture("registry").join("Cargo.toml"))
            .unwrap()
            .replace(
                "utf8-width = \"0.1.8\"",
                "utf8-width = { version = \"0.1.8\", optional = true }",
            );
        std::fs::write(project.join("Cargo.toml"), manifest).unwrap();
        std::fs::copy(
            fixture("registry").join("Cargo.lock"),
            project.join("Cargo.lock"),
        )
        .unwrap();
        std::fs::write(
            project.join("src/main.rs"),
            "fn main() { println!(\"optional absent\"); }\n",
        )
        .unwrap();
        let cargo_home = if damaged {
            let home = seed_registry_cache("utf8-width-0.1.8");
            std::fs::write(
                home.join("registry/src")
                    .join(CRATES_IO_CACHE_ID)
                    .join("utf8-width-0.1.8/src/lib.rs"),
                "pub fn forged() {}\n",
            )
            .unwrap();
            home
        } else {
            let home = scratch.join("empty-cargo-home");
            std::fs::create_dir_all(home.join("registry/src")).unwrap();
            std::fs::create_dir_all(home.join("registry/cache")).unwrap();
            home
        };
        for workers in ["1", "8"] {
            for command in ["check", "build", "run"] {
                let invoke = |activate| {
                    let mut cmd = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                    cmd.args([command, "-j", workers]);
                    if activate {
                        cmd.args(["--features", "utf8-width"]);
                    }
                    cmd.current_dir(&project)
                        .env("CARGO_HOME", &cargo_home)
                        .env("CARGO_TARGET_DIR", scratch.join("target"))
                        .output()
                        .unwrap()
                };
                let inactive = invoke(false);
                assert!(
                    inactive.status.success(),
                    "{}",
                    String::from_utf8_lossy(&inactive.stderr)
                );
                if command == "run" {
                    assert_eq!(inactive.stdout, b"optional absent\n");
                }
                let active = invoke(true);
                assert_eq!(active.status.code(), Some(101));
                let diagnostic = String::from_utf8_lossy(&active.stderr);
                assert!(
                    diagnostic.contains("local Cargo cache") || diagnostic.contains("checksum"),
                    "{diagnostic}"
                );
                assert!(!diagnostic.contains("Finished"), "{diagnostic}");
            }
        }
    }
}

#[test]
fn locked_registry_dependency_reports_checksum_mismatch_explicitly() {
    let scratch = target_dir("registry-checksum-mismatch");
    let project = scratch.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        std::fs::read_to_string(fixture("registry").join("Cargo.toml")).unwrap(),
    )
    .unwrap();
    let lock = std::fs::read_to_string(fixture("registry").join("Cargo.lock")).unwrap();
    std::fs::write(
        project.join("Cargo.lock"),
        lock.replace(
            "1292c0d970b54115d14f2492fe0170adf21d68a1de108eebc51c1df4f346a091",
            "0000000000000000000000000000000000000000000000000000000000000000",
        ),
    )
    .unwrap();
    std::fs::write(
        project.join("src/main.rs"),
        std::fs::read_to_string(fixture("registry").join("src/main.rs")).unwrap(),
    )
    .unwrap();
    let cargo_home = seed_registry_cache("utf8-width-0.1.8");
    let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args(["check"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .env("CARGO_HOME", cargo_home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("failed checksum validation"), "{stderr}");
    assert!(stderr.contains("utf8-width"), "{stderr}");
}

#[test]
fn locked_registry_dependency_rejects_mismatched_lock_entries() {
    let scratch = target_dir("registry-lock-mismatch");
    let project = scratch.join("project");
    std::fs::create_dir_all(project.join("src")).unwrap();
    std::fs::write(
        project.join("Cargo.toml"),
        std::fs::read_to_string(fixture("registry").join("Cargo.toml")).unwrap(),
    )
    .unwrap();
    let lock = std::fs::read_to_string(fixture("registry").join("Cargo.lock")).unwrap();
    std::fs::write(
        project.join("Cargo.lock"),
        lock.replace(" \"utf8-width\",\n", " \"utf8-width 0.1.9\",\n")
            .replace("version = \"0.1.8\"", "version = \"0.1.9\""),
    )
    .unwrap();
    std::fs::write(
        project.join("src/main.rs"),
        std::fs::read_to_string(fixture("registry").join("src/main.rs")).unwrap(),
    )
    .unwrap();
    let cargo_home = seed_registry_cache("utf8-width-0.1.8");
    let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args(["check"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .env("CARGO_HOME", cargo_home)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(101));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("locked registry package `utf8-width` 0.1.9"),
        "{stderr}"
    );
    assert!(stderr.contains("not cached"), "{stderr}");
}

#[test]
fn locked_registry_authority_rejects_forged_sources_and_locks() {
    let original_lock = std::fs::read_to_string(fixture("registry").join("Cargo.lock")).unwrap();
    let original_manifest =
        std::fs::read_to_string(fixture("registry").join("Cargo.toml")).unwrap();
    for (case, message) in [
        ("source", "unsupported locked registry source"),
        ("requirement", "does not satisfy manifest requirement"),
        ("missing-checksum", "missing a checksum"),
        ("ambiguous", "ambiguous"),
        (
            "modified-source",
            "registry source checksum validation failed",
        ),
        ("extra-source", "unauthenticated file"),
        ("wrong-cache", "not cached"),
    ] {
        let scratch = target_dir(&format!("registry-authority-{case}"));
        let project = scratch.join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        let mut lock = original_lock.clone();
        let mut manifest = original_manifest.clone();
        match case {
            "source" => {
                lock = lock.replace(
                    "github.com/rust-lang/crates.io-index",
                    "example.invalid/index",
                )
            }
            "requirement" => manifest = manifest.replace("0.1.8", "=0.1.9"),
            "missing-checksum" => {
                lock = lock
                    .lines()
                    .filter(|line| !line.starts_with("checksum"))
                    .collect::<Vec<_>>()
                    .join("\n")
            }
            "ambiguous" => lock.push_str(
                original_lock
                    .split_once("[[package]]\nname = \"utf8-width\"")
                    .map(|(_, tail)| format!("\n[[package]]\nname = \"utf8-width\"{tail}"))
                    .unwrap()
                    .as_str(),
            ),
            _ => {}
        }
        std::fs::write(project.join("Cargo.toml"), manifest).unwrap();
        std::fs::write(project.join("Cargo.lock"), lock).unwrap();
        std::fs::copy(
            fixture("registry").join("src/main.rs"),
            project.join("src/main.rs"),
        )
        .unwrap();
        // Every mutation is isolated from the user's licensed cache and other tests.
        let cargo_home = seed_registry_cache("utf8-width-0.1.8");
        let isolated = scratch.join("cargo-home");
        let status = Command::new("cp")
            .args(["-a"])
            .arg(&cargo_home)
            .arg(&isolated)
            .status()
            .unwrap();
        assert!(status.success());
        let sources = isolated
            .join("registry/src")
            .join(CRATES_IO_CACHE_ID)
            .join("utf8-width-0.1.8");
        match case {
            "modified-source" => std::fs::write(
                sources.join("src/lib.rs"),
                "pub fn get_width(_: u8) -> usize { 9 }\n",
            )
            .unwrap(),
            "extra-source" => std::fs::write(sources.join("src/forged.rs"), "").unwrap(),
            "wrong-cache" => {
                std::fs::rename(
                    isolated.join("registry/src").join(CRATES_IO_CACHE_ID),
                    isolated.join("registry/src/index.crates.io-forged"),
                )
                .unwrap();
            }
            _ => {}
        }
        for command in ["check", "build"] {
            let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .args([command])
                .current_dir(&project)
                .env("CARGO_HOME", &isolated)
                .env("CARGO_TARGET_DIR", scratch.join("target"))
                .output()
                .unwrap();
            assert_eq!(
                out.status.code(),
                Some(101),
                "{case}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                String::from_utf8_lossy(&out.stderr).contains(message),
                "{case}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(
                !scratch.join("target/debug/registry").exists(),
                "{case} published an executable"
            );
        }
    }
}

#[test]
fn locked_registry_unchanged_probe_boundaries() {
    for (name, version) in [("nix-base32", "0.2.0"), ("time-core", "0.1.2")] {
        let scratch = target_dir(&format!("registry-probe-{name}"));
        let project = scratch.join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        let stem = format!("{name}-{version}");
        let cargo_home = seed_registry_cache(&stem);
        let archive = cargo_home
            .join("registry/cache")
            .join(CRATES_IO_CACHE_ID)
            .join(format!("{stem}.crate"));
        let checksum = Command::new("sha256sum").arg(&archive).output().unwrap();
        assert!(checksum.status.success());
        let checksum = String::from_utf8(checksum.stdout).unwrap();
        let checksum = checksum.split_whitespace().next().unwrap();
        std::fs::write(project.join("Cargo.toml"), format!("[package]\nname = \"probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\n{name} = \"={version}\"\n")).unwrap();
        std::fs::write(project.join("Cargo.lock"), format!("version = 4\n[[package]]\nname = \"probe\"\nversion = \"0.1.0\"\ndependencies = [\"{name}\"]\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"{checksum}\"\n")).unwrap();
        let client = if name == "nix-base32" {
            "fn main() { let _ = nix_base32::to_nix_base32(&[1u8, 2u8]); }\n"
        } else {
            "fn main() { println!(\"{}\", time_core::convert::Day::per(time_core::convert::Week)); }\n"
        };
        std::fs::write(project.join("src/main.rs"), client).unwrap();
        for workers in ["1", "8"] {
            let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .args(["check", "-j", workers])
                .current_dir(&project)
                .env("CARGO_HOME", &cargo_home)
                .env("CARGO_TARGET_DIR", scratch.join("target"))
                .output()
                .unwrap();
            assert!(matches!(out.status.code(), Some(0 | 101)));
            eprintln!(
                "unchanged registry probe {name} {version} workers={workers} exit={:?}: {}",
                out.status.code(),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

#[test]
fn invalid_projects_are_rejected_explicitly() {
    let scratch = target_dir("invalid");
    std::fs::create_dir_all(scratch.join("src")).unwrap();
    std::fs::write(scratch.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    for (manifest, message) in [
        (
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nserde = \"1\"\n",
            "Cargo.lock does not pin",
        ),
        (
            "[package]\nversion = \"0.1.0\"\n",
            "package.name is required",
        ),
        ("[package\nname = \"x\"\n", "failed to parse manifest"),
        (
            "[package]\nname = \"x\"\nedition = \"2024\"\n[dependencies]\ny = { path = \"missing\" }\n",
            "missing",
        ),
    ] {
        std::fs::write(scratch.join("Cargo.toml"), manifest).unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .args(["check"])
            .current_dir(&scratch)
            .env("CARGO_TARGET_DIR", scratch.join("target"))
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(101));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(message), "{manifest}: {stderr}");
    }
    // A compile error in a dependency fails the dependent unit.
    std::fs::write(
        scratch.join("Cargo.toml"),
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(scratch.join("src/lib.rs"), "pub fn f() -> i32 { true }\n").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args(["check"])
        .current_dir(&scratch)
        .env("CARGO_TARGET_DIR", scratch.join("target"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(101));
    assert!(String::from_utf8_lossy(&out.stderr).contains("could not compile `x` (lib \"x\")"));
}
