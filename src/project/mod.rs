//! Existing-project support (G8): manifests, workspaces, targets, features,
//! profiles and a Cargo-compatible check/build/test/run driver.

pub(crate) mod capture;
pub mod cli;
pub mod driver;
pub mod inputs;
pub mod manifest;
mod registry_archive;
pub mod target_cfg;
pub mod toml;

/// Tests that model an otherwise unconfigured host must not inherit Cargo's
/// wrappers, flags or the developer's configuration. Isolate a child instead of
/// changing process-global environment or weakening production input checks.
#[cfg(test)]
fn in_clean_test_process(test: &str) -> bool {
    if std::env::var("JR_CLEAN_PROJECT_TEST").as_deref() == Ok(test) {
        return true;
    }
    let root = std::env::temp_dir().join(format!(
        "jr-clean-project-{}-{}",
        std::process::id(),
        test.replace("::", "-")
    ));
    std::fs::create_dir(&root).unwrap();
    std::fs::create_dir(root.join("home")).unwrap();
    std::fs::create_dir(root.join("cargo")).unwrap();
    let result = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("HOME", root.join("home"))
        .env("CARGO_HOME", root.join("cargo"))
        .env("JR_CLEAN_PROJECT_TEST", test)
        .current_dir(&root)
        .output();
    std::fs::remove_dir_all(root).unwrap();
    let output = result.unwrap();
    assert!(output.status.success(), "{output:?}");
    false
}
