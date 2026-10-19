//! Cargo project support: manifests, workspaces, path dependencies,
//! features, targets, `#[test]` harness and integration tests, driven
//! through `jr-rust check|build|test|run` and compared with stock Cargo.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
    Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args(args)
        .current_dir(fixture(project))
        .env("CARGO_TARGET_DIR", target_dir(project))
        .output()
        .unwrap()
}

fn stock(project: &str, args: &[&str]) -> Option<Output> {
    Command::new("cargo")
        .args(args)
        .current_dir(fixture(project))
        .env("CARGO_TARGET_DIR", target_dir(&format!("{project}-stock")))
        .output()
        .ok()
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
    let test = jr("workspace", &["test"]);
    assert!(
        test.status.success(),
        "{}",
        String::from_utf8_lossy(&test.stderr)
    );
    if let Some(expected) = stock("workspace", &["test"]).filter(|o| o.status.success()) {
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
    let test = jr("failing", &["test"]);
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
    // A plain build of the library ignores #[test] items entirely.
    assert!(jr("failing", &["check"]).status.success());
}

#[test]
fn invalid_projects_are_rejected_explicitly() {
    let scratch = target_dir("invalid");
    std::fs::create_dir_all(scratch.join("src")).unwrap();
    std::fs::write(scratch.join("src/lib.rs"), "pub fn f() {}\n").unwrap();
    for (manifest, message) in [
        (
            "[package]\nname = \"x\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\n",
            "registry dependencies are unsupported",
        ),
        (
            "[package]\nversion = \"0.1.0\"\n",
            "package.name is required",
        ),
        ("[package\nname = \"x\"\n", "failed to parse manifest"),
        (
            "[package]\nname = \"x\"\n[dependencies]\ny = { path = \"missing\" }\n",
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
        "[package]\nname = \"x\"\nversion = \"0.1.0\"\n",
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
