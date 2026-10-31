//! Licensed Blaze-derived v83 differential tests. Source-only delivery, run by root.
use super::*;

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "native-typed-v83-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(path.join("home")).unwrap();
        Self(path)
    }
    fn put(&self, name: &str, text: &str) {
        let path = self.0.join(name);
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
            build: Some(Build { packages: &[], workspace: true, excludes: &[], dev_units: true }),
            platform: Some(TargetPlatform { triple: "x86_64-unknown-linux-gnu", cfg: &[] }),
            offline: true,
            store: None,
            rustc_version: Some("1.101.0-nightly"),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn same_read(a: &Read, b: &Read) {
    let (a_sum, b_sum) = (&a.summary, &b.summary);
    assert_eq!(a_sum.package_id(), b_sum.package_id());
    // Summary Eq/Hash are ID-only. Dependency Eq compares every inner field.
    assert_eq!(a_sum.dependencies(), b_sum.dependencies());
    assert_eq!(a_sum.features(), b_sum.features());
    assert_eq!(a_sum.links(), b_sum.links());
    assert_eq!(a_sum.checksum(), b_sum.checksum());
    assert_eq!(a_sum.rust_version(), b_sum.rust_version());
    assert_eq!(a_sum.pubtime(), b_sum.pubtime());
    assert_eq!(a.lib, b.lib);
    assert_eq!(a.proc_macro_lib, b.proc_macro_lib);
    assert_eq!(a.any_proc_macro, b.any_proc_macro);
    assert_eq!(a.build_script, b.build_script);
    assert_eq!(a.bins, b.bins);
    assert_eq!(a.license, b.license);
    assert_eq!(a.repository, b.repository);
}
fn chain(e: &anyhow::Error) -> Vec<String> {
    e.chain().map(ToString::to_string).collect()
}
fn same_package(p: &SerializedPackage<'_>) {
    let json = serde_json::to_value(p).unwrap();
    match (summary::read(&json), summary::read_typed(p)) {
        (Ok(mut a), Ok(mut b)) => {
            same_read(&a, &b);
            a.summary.set_checksum("fixture-checksum".into());
            b.summary.set_checksum("fixture-checksum".into());
            a.summary.set_pubtime("2026-10-31T00:00:00Z".into());
            b.summary.set_pubtime("2026-10-31T00:00:00Z".into());
            same_read(&a, &b);
        }
        (Err(a), Err(b)) => assert_eq!(chain(&a), chain(&b)),
        _ => panic!("summary success/error mismatch"),
    }
}

#[test]
fn filesystem_exported_summaries_preserve_complete_state() {
    let f = Fixture::new();
    f.put("Cargo.toml", r#"[package]
name = "typed-app"
version = "1.2.3-beta.1"
edition = "2021"
license = "MIT"
repository = "https://example.invalid/repo"
links = "typed_native"
rust-version = "1.80"
[features]
default = ["alias"]
extra = ["alias?/extra"]
[dependencies]
alias = { package = "dep", path = "dep", version = "*", optional = true, default-features = false, features = ["extra"] }
reg = { package = "registry-dep", version = "1" }
gitdep = { git = "https://example.invalid/project", branch = "topic" }
[dev-dependencies]
dev = { package = "dep", path = "dep" }
[build-dependencies]
build = { package = "dep", path = "dep" }
[target.'cfg(unix)'.dependencies]
platform = { package = "dep", path = "dep", optional = true }
[[bin]]
name = "needs-extra"
path = "src/main.rs"
required-features = ["extra", "alias/extra"]
[lib]
crate-type = ["rlib", "cdylib", "staticlib"]
"#);
    f.put("src/lib.rs", "pub fn value() {}\n");
    f.put("src/main.rs", "fn main() {}\n");
    f.put("build.rs", "fn main() {}\n");
    f.put("dep/Cargo.toml", "[package]\nname='dep'\nversion='0.1.0'\n[features]\nextra=[]\n");
    f.put("dep/src/lib.rs", "");
    let ctx = Context::new(Config::load(&f.0, f.asked().env, &[]).unwrap());
    let path = f.0.join("Cargo.toml");
    let ws = Workspace::new(&path, &ctx).unwrap();
    for (path, manifest) in ws.members() {
        let mut p = export_package(path, manifest);
        same_package(&p);
        p.set_source("registry+https://github.com/rust-lang/crates.io-index".into());
        same_package(&p);
        p.set_source("git+https://example.invalid/project?branch=topic#0123456789012345678901234567890123456789".into());
        same_package(&p);
        p.set_source("not a valid source URL".into());
        same_package(&p);
    }
}

fn workspace(f: &Fixture) {
    f.put("Cargo.toml", "[workspace]\nmembers=['app','dep']\nresolver='2'\n");
    f.put(".cargo/config.toml", "[build]\ntarget='x86_64-unknown-linux-gnu'\n");
    f.put(
        "app/Cargo.toml",
        r#"[package]
name='app'
version='0.1.0'
edition='2021'
[dependencies]
alias={package='dep',path='../dep',optional=true,features=['extra']}
[features]
default=['alias']
extra=['alias?/extra']
[build-dependencies]
alias={package='dep',path='../dep'}
[dev-dependencies]
alias={package='dep',path='../dep'}
[target.'cfg(unix)'.dependencies]
alias={package='dep',path='../dep'}
[[bin]]
name='gated'
path='src/main.rs'
required-features=['extra','alias/extra']
[[test]]
name='integration'
path='tests/integration.rs'
harness=false
[lib]
crate-type=['rlib','cdylib','staticlib']
"#,
    );
    for path in [
        "app/src/lib.rs",
        "app/src/main.rs",
        "app/tests/integration.rs",
        "app/examples/sample.rs",
        "app/build.rs",
        "dep/src/lib.rs",
    ] {
        f.put(path, "fn main() {}\n");
    }
    f.put(
        "dep/Cargo.toml",
        "[package]\nname='dep'\nversion='0.1.0'\nedition='2021'\n[features]\nextra=[]\n",
    );
}
fn compare_resolved(asked: &Asked<'_>) {
    // Own complete Read values across callbacks. No Summary equality shortcuts.
    let original = resolving_with_mode(asked, None, None, false, |r, s, ids, _| {
        let json = described(asked, &r, s, ids)?.json;
        Ok((
            r.packages
                .iter()
                .map(|(id, read)| (*id, clone_read(read)))
                .collect::<FxHashMap<_, _>>(),
            json,
        ))
    })
    .unwrap();
    let native = resolving_with_mode(asked, None, None, true, |r, s, ids, _| {
        let json = described(asked, &r, s, ids)?.json;
        Ok((
            r.packages
                .iter()
                .map(|(id, read)| (*id, clone_read(read)))
                .collect::<FxHashMap<_, _>>(),
            json,
        ))
    })
    .unwrap();
    assert_eq!(original.1, native.1, "full metadata JSON bytes/order");
    assert_eq!(original.0.len(), native.0.len());
    for (id, read) in &original.0 {
        same_read(read, &native.0[id]);
    }
}
fn clone_read(r: &Read) -> Read {
    Read {
        summary: r.summary.clone(),
        lib: r.lib,
        proc_macro_lib: r.proc_macro_lib,
        any_proc_macro: r.any_proc_macro,
        build_script: r.build_script,
        bins: r.bins.clone(),
        license: r.license.clone(),
        repository: r.repository.clone(),
    }
}
#[test]
fn filesystem_programs_json_errors_features_and_revisions() {
    let f = Fixture::new();
    workspace(&f);
    for revision in 0..2 {
        if revision == 1 {
            f.put("app/examples/second.rs", "fn main() {}\n");
        }
        for mode in 0..4 {
            let mut asked = f.asked();
            let cfg = vec!["unix".parse().unwrap()];
            asked.platform = Some(TargetPlatform { triple: "x86_64-unknown-linux-gnu", cfg: &cfg });
            asked.all_features = mode == 1;
            asked.no_default_features = mode == 2;
            let explicit = vec!["extra".to_owned()];
            if mode == 3 {
                asked.features = &explicit;
            }
            compare_resolved(&asked);
            for targets in [
                Targets::default(),
                Targets { test: Some(Tests::default()), ..Targets::default() },
                Targets { bin: Some("gated"), ..Targets::default() },
                Targets { example: Some("sample"), ..Targets::default() },
                Targets { lib: true, ..Targets::default() },
                Targets {
                    test: Some(Tests { lib: false, test: Some("integration") }),
                    ..Targets::default()
                },
            ] {
                match (programs(&asked, targets), programs_native_typed_inner(&asked, targets)) {
                    (Ok((json, a)), Ok(b)) => {
                        assert_eq!(a, b);
                        assert_eq!(
                            serde_json::to_string(&a).unwrap(),
                            serde_json::to_string(&b).unwrap(),
                            "program stdout JSON"
                        );
                        assert_eq!(
                            json,
                            metadata(&asked).unwrap(),
                            "independent original JSON unchanged"
                        );
                    }
                    (Err(a), Err(b)) => assert_eq!(chain(&a), chain(&b)),
                    _ => panic!("program success/error mismatch"),
                }
            }
        }
    }
    f.put("app/Cargo.toml", "[package\nnot valid\n");
    let asked = f.asked();
    let a = programs(&asked, Targets::default()).unwrap_err();
    let b = programs_native_typed_inner(&asked, Targets::default()).unwrap_err();
    assert_eq!(chain(&a), chain(&b));
}

#[test]
fn serialized_unusual_library_kinds_and_proc_macros_are_not_reinterpreted() {
    for kinds in ["['proc-macro']", "['dylib']", "['rlib','cdylib','staticlib']", "['bin']"] {
        let f = Fixture::new();
        f.put("Cargo.toml", &format!("[package]\nname='odd-kinds'\nversion='0.1.0'\nedition='2021'\n[lib]\ncrate-type={kinds}\n"));
        f.put("src/lib.rs", "");
        let ctx = Context::new(Config::load(&f.0, f.asked().env, &[]).unwrap());
        let manifest = f.0.join("Cargo.toml");
        let ws = Workspace::new(&manifest, &ctx).unwrap();
        for (path, m) in ws.members() {
            same_package(&export_package(path, m));
        }
    }
}

#[test]
fn source_erasure_and_independent_oracle_boundaries() {
    let source = include_str!("../../src/lib.rs");
    let native = source
        .split("fn programs_native_typed_inner(")
        .nth(1)
        .unwrap()
        .split("pub struct Units")
        .next()
        .unwrap();
    assert!(!native.contains("serde_json"));
    assert!(!native.contains("described("));
    assert!(native.contains("resolving_with_mode(asked, None, None, true"));
    let original = source
        .split("pub fn programs(")
        .nth(1)
        .unwrap()
        .split("pub fn native_typed_planning_enabled")
        .next()
        .unwrap();
    assert!(original.contains("serde_json::from_str(&json)"));
    assert!(!original.contains("read_typed"));
    let summary = include_str!("../../src/summary.rs");
    let oracle = summary
        .split("pub(crate) fn read(")
        .nth(1)
        .unwrap()
        .split("pub(crate) fn read_typed(")
        .next()
        .unwrap();
    assert!(!oracle.contains("planning()"));
}
