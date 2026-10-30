//! Differential oracle for the native programs-only API. No compiler or cargo subprocess.
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use blaze_metadata::{Asked, Build, Kind, Program, Targets, Tests};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let root = std::env::temp_dir().join(format!(
            "resident-typed-planning-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&root).unwrap();
        Self(root)
    }
    fn put(&self, path: &str, text: &str) {
        let path = self.0.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    fn asked(&self) -> Asked<'_> {
        Asked {
            cwd: &self.0,
            env: vec![("CARGO_HOME".into(), self.0.join("home").display().to_string())],
            manifest_path: None,
            config: &[],
            features: &[],
            all_features: false,
            no_default_features: false,
            build: Some(Build { packages: &[], workspace: false, excludes: &[], dev_units: true }),
            platform: None,
            offline: true,
            store: None,
            rustc_version: Some("1.99.0"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn outcome(result: anyhow::Result<Vec<Program>>) -> Result<Vec<Program>, (i32, String)> {
    // The resident caller preserves its existing refusal path and maps every error to 101.
    result.map_err(|err| (101, format!("{err:#}")))
}
fn compare(asked: &Asked<'_>, targets: Targets<'_>) -> Result<Vec<Program>, (i32, String)> {
    // Compare complete vectors and errors, then repeat in reverse order while locked.
    let old = outcome(blaze_metadata::programs(asked, targets).map(|(_, programs)| programs));
    let typed = outcome(blaze_metadata::programs_only(asked, targets));
    assert_eq!(typed, old);
    assert_eq!(outcome(blaze_metadata::programs_only(asked, targets)), old);
    assert_eq!(outcome(blaze_metadata::programs(asked, targets).map(|(_, p)| p)), old);
    typed
}

#[test]
fn typed_programs_leave_metadata_json_bytes_unchanged() {
    let f = workspace();
    for resolver in ["1", "2", "3"] {
        f.put("Cargo.toml", &format!(
            "[workspace]\nmembers=['zeta','alpha']\ndefault-members=['zeta']\nresolver='{resolver}'\n"
        ));
        let asked = f.asked();
        let targets = Targets { test: Some(Tests::default()), ..Targets::default() };
        let (before, old) = blaze_metadata::programs(&asked, targets).unwrap();
        assert_eq!(blaze_metadata::programs_only(&asked, targets).unwrap(), old);
        let (after, _) = blaze_metadata::programs(&asked, targets).unwrap();
        assert_eq!(before.as_bytes(), after.as_bytes());
        assert_eq!(before.as_bytes(), blaze_metadata::metadata(&asked).unwrap().as_bytes());
    }
}

fn workspace() -> Fixture {
    let f = Fixture::new();
    f.put(
        "Cargo.toml",
        "[workspace]\nmembers=['zeta','alpha']\ndefault-members=['zeta']\nresolver='2'\n",
    );
    f.put(
        "zeta/Cargo.toml",
        r#"
[package]
name = "zeta"
version = "0.1.0"
edition = "2021"
[features]
default = ["enabled"]
enabled = []
gated = []
[dependencies]
alpha = { path = "../alpha", features = ["shared"] }
[lib]
crate-type = ["rlib", "cdylib", "staticlib"]
harness = false
[[bin]]
name = "z-main"
path = "src/main.rs"
[[bin]]
name = "gated-bin"
path = "src/gated.rs"
required-features = ["gated", "enabled"]
harness = false
[[bin]]
name = "dep-feature"
path = "src/dep.rs"
required-features = ["alpha/shared"]
test = false
[[example]]
name = "normal"
path = "examples/normal.rs"
[[example]]
name = "tested"
path = "examples/tested.rs"
test = true
harness = false
[[example]]
name = "library"
path = "examples/library.rs"
crate-type = ["rlib"]
[[test]]
name = "integration"
path = "tests/integration.rs"
harness = false
[[test]]
name = "gated-test"
path = "tests/gated.rs"
required-features = ["gated"]
[[bench]]
name = "bench"
path = "benches/bench.rs"
"#,
    );
    f.put("alpha/Cargo.toml", "[package]\nname='alpha'\nversion='0.1.0'\nedition='2021'\n[features]\nshared=[]\n[lib]\nproc-macro=true\n");
    for path in [
        "zeta/src/lib.rs",
        "zeta/src/main.rs",
        "zeta/src/gated.rs",
        "zeta/src/dep.rs",
        "zeta/examples/normal.rs",
        "zeta/examples/tested.rs",
        "zeta/examples/library.rs",
        "zeta/tests/integration.rs",
        "zeta/tests/gated.rs",
        "zeta/benches/bench.rs",
        "alpha/src/lib.rs",
    ] {
        f.put(path, "");
    }
    f
}

#[test]
fn typed_programs_match_json_targets_features_and_package_selection() {
    let f = workspace();
    let packages = vec!["alpha".into(), "zeta".into()];
    let excludes = vec!["alpha".into()];
    let features = vec!["zeta/gated".into()];
    let selections = [
        Targets::default(),
        Targets { lib: true, ..Targets::default() },
        Targets { bin: Some("z-main"), ..Targets::default() },
        Targets { bin: Some("gated-bin"), ..Targets::default() },
        Targets { bin: Some("dep-feature"), ..Targets::default() },
        Targets { example: Some("normal"), ..Targets::default() },
        Targets { example: Some("library"), ..Targets::default() },
        Targets { test: Some(Tests::default()), ..Targets::default() },
        Targets { test: Some(Tests { lib: true, test: None }), ..Targets::default() },
        Targets {
            test: Some(Tests { lib: false, test: Some("integration") }),
            ..Targets::default()
        },
        Targets {
            test: Some(Tests { lib: false, test: Some("gated-test") }),
            ..Targets::default()
        },
        Targets { example: Some("tested"), test: Some(Tests::default()), ..Targets::default() },
        Targets { bin: Some("gated-bin"), test: Some(Tests::default()), ..Targets::default() },
        Targets { bin: Some("absent"), ..Targets::default() },
    ];
    for selection in 0..4 {
        for feature_mode in 0..4 {
            let mut asked = f.asked();
            asked.build = Some(Build {
                packages: if selection == 1 { &packages } else { &[] },
                workspace: selection >= 2,
                excludes: if selection == 3 { &excludes } else { &[] },
                dev_units: true,
            });
            asked.features = if feature_mode == 1 { &features } else { &[] };
            asked.all_features = feature_mode == 2;
            asked.no_default_features = feature_mode == 3;
            for targets in selections {
                let _ = compare(&asked, targets);
            }
        }
    }
    let mut asked = f.asked();
    asked.all_features = true;
    let programs =
        compare(&asked, Targets { test: Some(Tests::default()), ..Targets::default() }).unwrap();
    assert!(programs.iter().any(|p| p.kind == Kind::Bin && !p.test));
    assert!(programs.iter().any(|p| p.kind == Kind::Test && !p.harness));
    assert!(programs.iter().any(|p| p.kind == Kind::Example && !p.test));
    assert!(programs.iter().any(|p| p.kind == Kind::Example && p.test && !p.harness));
    assert!(programs.iter().any(|p| p.kind == Kind::Lib && p.test && !p.harness));
    assert!(programs.iter().any(|p| p.name == "dep-feature" && !p.test));
    assert!(!programs.iter().any(|p| p.name == "library" || p.name == "bench"));
    let libraries = compare(&asked, Targets { lib: true, ..Targets::default() }).unwrap();
    assert_eq!(
        libraries.iter().map(|p| p.kind).collect::<Vec<_>>(),
        [Kind::Cdylib, Kind::Staticlib]
    );
}

#[test]
fn typed_programs_match_distinct_legal_library_types_and_invalid_combination() {
    let f = workspace();
    let manifest = std::fs::read_to_string(f.0.join("zeta/Cargo.toml")).unwrap();
    let original_types = "crate-type = [\"rlib\", \"cdylib\", \"staticlib\"]";
    assert_eq!(manifest.matches(original_types).count(), 1);
    let asked = f.asked();
    let library = Targets { lib: true, ..Targets::default() };
    let libraries = compare(&asked, library).unwrap();
    assert_eq!(
        libraries.iter().map(|p| p.kind).collect::<Vec<_>>(),
        [Kind::Cdylib, Kind::Staticlib]
    );

    // A dylib cannot coexist with a cdylib. Exercise it as a distinct valid revision,
    // rather than allowing an invalid common fixture to short-circuit every success gate.
    f.put(
        "zeta/Cargo.toml",
        &manifest.replace(original_types, "crate-type = [\"rlib\", \"dylib\"]"),
    );
    let libraries = compare(&asked, library).unwrap();
    assert_eq!(libraries.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Dylib]);
    let defaults = compare(&asked, Targets::default()).unwrap();
    assert!(defaults.iter().any(|p| p.kind == Kind::Dylib && !p.test));
    let tests = compare(
        &asked,
        Targets { test: Some(Tests { lib: true, test: None }), ..Targets::default() },
    )
    .unwrap();
    assert_eq!(tests.iter().map(|p| p.kind).collect::<Vec<_>>(), [Kind::Lib]);
    assert!(tests[0].test && !tests[0].harness);

    // Keep the rejected v7 combination as a deliberate exact-error differential case.
    f.put(
        "zeta/Cargo.toml",
        &manifest.replace(
            original_types,
            "crate-type = [\"rlib\", \"cdylib\", \"staticlib\", \"dylib\"]",
        ),
    );
    let err = compare(&asked, library).unwrap_err();
    assert_eq!(
        err,
        (
            101,
            format!(
                "failed to load manifest for workspace member `{}`\nreferenced by workspace at `{}`: failed to parse manifest at `{}`: library `zeta` cannot set the crate type of both `dylib` and `cdylib`",
                f.0.join("zeta").display(),
                f.0.join("Cargo.toml").display(),
                f.0.join("zeta/Cargo.toml").display()
            )
        )
    );
}

#[test]
fn typed_programs_match_json_from_cold_and_after_manifest_edits() {
    let f = workspace();
    let targets = Targets { test: Some(Tests::default()), ..Targets::default() };
    let asked = f.asked();
    let typed = outcome(blaze_metadata::programs_only(&asked, targets));
    assert!(typed.is_ok());
    assert_eq!(typed, outcome(blaze_metadata::programs(&asked, targets).map(|(_, p)| p)));
    // No manual memoization: target/feature edits must be read on the next request.
    f.put(
        "zeta/Cargo.toml",
        "[package]\nname='zeta'\nversion='0.1.0'\nedition='2021'\n[lib]\nharness=true\n",
    );
    let edited = compare(&asked, targets).unwrap();
    assert!(edited.iter().any(|p| p.kind == Kind::Lib && p.harness));
    assert!(!edited.iter().any(|p| p.name == "gated-bin"));
    f.put("zeta/Cargo.toml", "[package\n");
    assert!(compare(&asked, targets).is_err());
}

#[test]
fn typed_programs_match_resolver_configuration_platform_and_errors() {
    let f = workspace();
    let configs = [
        vec![],
        vec![
            "unstable.feature-unification=true".into(),
            "resolver.feature-unification='workspace'".into(),
        ],
        vec![
            "unstable.feature-unification=true".into(),
            "resolver.feature-unification='package'".into(),
        ],
    ];
    let packages = vec!["alpha".into(), "zeta".into()];
    for (index, config) in configs.iter().enumerate() {
        let mut asked = f.asked();
        asked.config = config;
        asked.build.as_mut().unwrap().packages = &packages;
        let result = compare(&asked, Targets::default());
        if index == 2 {
            assert!(result.unwrap_err().1.contains("resolver.feature-unification = \"package\""));
            asked.build.as_mut().unwrap().packages = &packages[1..];
            compare(&asked, Targets::default()).unwrap();
        } else {
            result.unwrap();
        }
    }
    let mut asked = f.asked();
    asked.platform =
        Some(blaze_metadata::TargetPlatform { triple: "x86_64-unknown-linux-gnu", cfg: &[] });
    asked.build.as_mut().unwrap().dev_units = false;
    compare(&asked, Targets::default()).unwrap();
    let alpha = f.0.join("alpha");
    asked.cwd = &alpha;
    compare(&asked, Targets { test: Some(Tests::default()), ..Targets::default() }).unwrap();
    let mut asked = f.asked();
    let err =
        compare(&asked, Targets { bin: Some("gated-bin"), ..Targets::default() }).unwrap_err();
    assert_eq!(err.0, 101);
    assert_eq!(
        err.1,
        "target `gated-bin` in package `zeta` requires the features: `gated`\nConsider enabling them by passing, e.g., `--features=\"gated\"`"
    );
    let unknown = vec!["unknown".into()];
    asked.build.as_mut().unwrap().packages = &unknown;
    assert!(compare(&asked, Targets::default()).is_err());
    let invalid_features = vec!["not-a-feature".into()];
    asked.build.as_mut().unwrap().packages = &[];
    asked.features = &invalid_features;
    assert!(compare(&asked, Targets::default()).is_err());
    asked.build = None;
    assert_eq!(
        compare(&asked, Targets::default()).unwrap_err(),
        (101, "the programs are of a build".into())
    );
}
