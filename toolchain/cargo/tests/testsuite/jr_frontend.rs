//! Native workspace-only frontend policy acceptance, including fingerprints.

use crate::prelude::*;
use cargo_test_support::{Project, project};
use std::process::Output;

fn fixture() -> Project {
    project()
        .file("Cargo.toml", "[workspace]\nmembers=['member']\nexclude=['external']\n[package]\nname='frontend-app'\nversion='0.1.0'\nedition='2021'\n[dependencies]\nmember={path='member'}\nexternal={path='external'}\n")
        .file("src/lib.rs", "pub fn value() -> u32 { member::value() + external::value() }")
        .file("build.rs", "fn main() { println!(\"cargo:rerun-if-changed=build.rs\"); }")
        .file("member/Cargo.toml", "[package]\nname='member'\nversion='0.1.0'\nedition='2021'\n")
        .file("member/src/lib.rs", "pub fn value() -> u32 { 1 }")
        .file("external/Cargo.toml", "[package]\nname='external'\nversion='0.1.0'\nedition='2021'\n")
        .file("external/src/lib.rs", "pub fn value() -> u32 { 2 }")
        .build()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn invocation<'a>(text: &'a str, name: &str) -> &'a str {
    text.lines()
        .find(|line| line.contains("Running `") && line.contains(&format!("--crate-name {name} ")))
        .unwrap_or_else(|| panic!("missing rustc invocation for {name}: {text}"))
}

#[cargo_test(nightly, reason = "frontend threads require actual nightly rustc")]
fn workspace_only_and_switch_rebuilds_only_affected_units() {
    let p = fixture();
    let output = p
        .cargo("check -v")
        .env("JUSTRUST_FRONTEND_THREADS", "4")
        .exec_with_output()
        .unwrap();
    let text = stderr(&output);
    for name in ["frontend_app", "member"] {
        assert!(invocation(&text, name).contains("-Zthreads=4"), "{text}");
    }
    for name in ["external", "build_script_build"] {
        assert!(!invocation(&text, name).contains("-Zthreads="), "{text}");
    }

    let output = p
        .cargo("check -v")
        .env("JUSTRUST_FRONTEND_THREADS", "2")
        .exec_with_output()
        .unwrap();
    let text = stderr(&output);
    for name in ["frontend_app", "member"] {
        assert!(invocation(&text, name).contains("-Zthreads=2"), "{text}");
    }
    assert!(
        !text.contains("--crate-name external "),
        "dependency rebuilt: {text}"
    );
    assert!(
        !text.contains("--crate-name build_script_build "),
        "build script rebuilt: {text}"
    );

    let output = p
        .cargo("check -v")
        .env("JUSTRUST_FRONTEND_THREADS", "2")
        .exec_with_output()
        .unwrap();
    assert!(
        !stderr(&output).contains("--crate-name "),
        "unchanged opt-in not fresh"
    );
}

#[cargo_test(nightly, reason = "frontend controls require actual nightly rustc")]
fn explicit_rustflags_and_cargo_rustc_controls_win() {
    let p = fixture();
    let output = p
        .cargo("check -v")
        .env("JUSTRUST_FRONTEND_THREADS", "8")
        .env("RUSTFLAGS", "-Z unstable-options --jobs-frontend=1")
        .exec_with_output()
        .unwrap();
    assert!(!stderr(&output).contains("-Zthreads=8"));

    let output = p
        .cargo("rustc -v --lib -- -Zthreads=1")
        .env("JUSTRUST_FRONTEND_THREADS", "8")
        .exec_with_output()
        .unwrap();
    let text = stderr(&output);
    let app = invocation(&text, "frontend_app");
    assert!(app.contains("-Zthreads=1"), "{text}");
    assert!(!app.contains("-Zthreads=8"), "{text}");
}

#[cargo_test]
fn invalid_or_disabled_setting_does_not_add_unstable_flags() {
    let p = fixture();
    for setting in ["0", "256", "auto"] {
        let output = p
            .cargo("check -v")
            .env("JUSTRUST_FRONTEND_THREADS", setting)
            .exec_with_output()
            .unwrap();
        assert!(!stderr(&output).contains("-Zthreads="));
    }
}

#[cargo_test(nightly, reason = "frontend threads require actual nightly rustc")]
fn cargo_env_configuration_uses_normal_override_and_force_precedence() {
    let p = fixture();
    p.change_file(
        ".cargo/config.toml",
        "[env]\nJUSTRUST_FRONTEND_THREADS='4'\n",
    );
    let output = p
        .cargo("check -v")
        .env_remove("JUSTRUST_FRONTEND_THREADS")
        .exec_with_output()
        .unwrap();
    assert!(invocation(&stderr(&output), "frontend_app").contains("-Zthreads=4"));

    let output = p
        .cargo("check -v")
        .env("JUSTRUST_FRONTEND_THREADS", "2")
        .exec_with_output()
        .unwrap();
    assert!(invocation(&stderr(&output), "frontend_app").contains("-Zthreads=2"));

    p.change_file(
        ".cargo/config.toml",
        "[env]\nJUSTRUST_FRONTEND_THREADS={value='6',force=true}\n",
    );
    let output = p
        .cargo("check -v")
        .env("JUSTRUST_FRONTEND_THREADS", "2")
        .exec_with_output()
        .unwrap();
    assert!(invocation(&stderr(&output), "frontend_app").contains("-Zthreads=6"));
}
