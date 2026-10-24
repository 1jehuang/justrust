//! Native target-conditioned dependency admission and selected-edge execution.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::project::manifest;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir)
            .join(format!(
                "jr-target-deps-{}-{stamp}-{tag}",
                std::process::id()
            ));
        fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn write(&self, path: &str, source: &str) {
        let path = self.0.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, source).unwrap();
    }
    fn manifest(&self) -> PathBuf {
        self.0.join("app/Cargo.toml")
    }
    fn app(&self, extra: &str, source: &str) {
        self.write(
            "app/Cargo.toml",
            &format!("[package]\nname=\"app\"\nversion=\"0.1.0\"\nedition=\"2024\"\n{extra}"),
        );
        self.write("app/src/main.rs", source);
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn native(path: &Path, args: &[&str], workers: usize) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args(args)
        .arg("--manifest-path")
        .arg(path)
        .arg("-j")
        .arg(workers.to_string())
        .output()
        .unwrap()
}

#[test]
fn selected_platform_edge_and_feature_match_stock() {
    let scratch = Scratch::new("selection");
    scratch.app(
        r#"
[features]
default = ["choose"]
choose = ["platform/selected"]
[target.'cfg(any())'.dependencies]
platform = { package = "active", path = "../active", optional = true, default-features = false }
excluded = { package = "inactive", path = "../inactive" }
[target.'cfg(unix)'.dependencies]
platform = { package = "active", path = "../active", optional = true, default-features = false }
"#,
        "fn main(){println!(\"{}\",platform::value());}\n",
    );
    scratch.write(
        "active/Cargo.toml",
        r#"
[package]
name="active"
version="0.1.0"
edition="2024"
[features]
selected=[]
"#,
    );
    scratch.write(
        "active/src/lib.rs",
        r#"
#[cfg(feature="selected")] pub fn value()->i32 {7}
#[cfg(not(feature="selected"))] pub fn value()->i32 {3}
"#,
    );
    scratch.write(
        "inactive/Cargo.toml",
        "[package]\nname=\"inactive\"\nversion=\"0.1.0\"\nedition=\"2021\"\n",
    );
    scratch.write(
        "inactive/src/lib.rs",
        "compile_error!(\"inactive target must not compile\");\n",
    );
    // The same optional edge appears in both conditions; an inactive first
    // declaration must not mask the active one during feature propagation.
    let expected = Command::new("cargo")
        .args(["run", "--offline", "--quiet", "--manifest-path"])
        .arg(scratch.manifest())
        .env("CARGO_TARGET_DIR", scratch.0.join("stock-target"))
        .output()
        .unwrap();
    assert!(
        expected.status.success(),
        "{}",
        String::from_utf8_lossy(&expected.stderr)
    );
    assert_eq!(expected.stdout, b"7\n");
    let workspace = manifest::load(&scratch.manifest()).unwrap();
    let root = workspace.default_members[0];
    let features = manifest::resolve_features(&workspace, &[(root, vec![], true)], false).unwrap();
    let active = workspace
        .packages
        .iter()
        .position(|p| p.name == "active")
        .unwrap();
    let inactive = workspace
        .packages
        .iter()
        .position(|p| p.name == "inactive")
        .unwrap();
    assert!(features.enabled[active].contains("selected"));
    assert!(features.enabled[inactive].is_empty());
    for workers in [1, 8] {
        let checked = native(&scratch.manifest(), &["check"], workers);
        assert!(
            checked.status.success(),
            "{}",
            String::from_utf8_lossy(&checked.stderr)
        );
        let actual = native(&scratch.manifest(), &["run"], workers);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
        assert_eq!(actual.stdout, expected.stdout, "workers={workers}");
    }
}

#[test]
fn inactive_registry_and_build_edges_do_not_demand_sources() {
    let scratch = Scratch::new("inactive");
    scratch.app(
        r#"
[target.'cfg(windows)'.dependencies]
unavailable = "99.0"
[target.'cfg(windows)'.build-dependencies]
missing_build_tool = "99.0"
[target.'cfg(not(miri))'.dev-dependencies]
unavailable_dev = "99.0"
"#,
        "fn main(){}\n",
    );
    let workspace = manifest::load(&scratch.manifest()).unwrap();
    let root = workspace.default_members[0];
    let features = manifest::resolve_features(&workspace, &[(root, vec![], true)], false).unwrap();
    let mut unresolved = Vec::new();
    assert!(
        manifest::visible_dependencies(
            &workspace,
            root,
            false,
            &features.active_deps[root],
            &mut unresolved
        )
        .is_empty()
    );
    assert!(unresolved.is_empty(), "{unresolved:?}");
    for workers in [1, 8] {
        let actual = native(&scratch.manifest(), &["check"], workers);
        assert!(
            actual.status.success(),
            "{}",
            String::from_utf8_lossy(&actual.stderr)
        );
    }
}

#[test]
fn malformed_conditions_and_active_build_scripts_reject() {
    for (tag, selector, section, diagnostic) in [
        (
            "arity",
            "cfg(any(unix, not()))",
            "dependencies",
            "invalid cfg",
        ),
        (
            "profile",
            "cfg(debug_assertions)",
            "dependencies",
            "crate or profile",
        ),
        (
            "features",
            "cfg(feature = \"x\")",
            "dependencies",
            "crate or profile",
        ),
        (
            "build",
            "cfg(unix)",
            "build-dependencies",
            "build-dependencies",
        ),
    ] {
        let scratch = Scratch::new(tag);
        scratch.app(
            &format!("[target.'{selector}'.{section}]\ndep = \"1\"\n"),
            "fn main(){}\n",
        );
        let error = manifest::load(&scratch.manifest()).unwrap_err().to_string();
        assert!(error.contains(diagnostic), "{tag}: {error}");
    }
}

#[test]
fn inactive_path_still_requires_valid_package_identity() {
    let scratch = Scratch::new("path");
    scratch.app(
        "[target.'cfg(windows)'.dependencies]\nmissing = {path=\"../missing\"}\n",
        "fn main(){}\n",
    );
    assert!(manifest::load(&scratch.manifest()).is_err());
}

#[test]
fn edition_support_is_required_only_for_compiled_dependencies() {
    for (tag, selector, edition, diagnostic) in [
        (
            "active-old",
            "cfg(unix)",
            "2018",
            "supports only editions 2021 and 2024",
        ),
        (
            "inactive-invalid",
            "cfg(windows)",
            "2099",
            "invalid Rust edition",
        ),
    ] {
        let scratch = Scratch::new(tag);
        scratch.app(
            &format!("[target.'{selector}'.dependencies]\nother = {{path=\"../other\"}}\n"),
            "fn main(){}\n",
        );
        scratch.write(
            "other/Cargo.toml",
            &format!("[package]\nname=\"other\"\nversion=\"0.1.0\"\nedition=\"{edition}\"\n"),
        );
        scratch.write("other/src/lib.rs", "pub fn value()->i32{7}\n");
        for workers in [1, 8] {
            let actual = native(&scratch.manifest(), &["check"], workers);
            assert!(!actual.status.success(), "{tag}: unexpectedly accepted");
            assert!(
                String::from_utf8_lossy(&actual.stderr).contains(diagnostic),
                "{tag}: {}",
                String::from_utf8_lossy(&actual.stderr)
            );
        }
    }
}
