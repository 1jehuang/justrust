//! Native differential fixtures. The oracle below is the complete accepted v80
//! read_manifest body, not a call to the new normalization helper.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use super::*;
use anyhow::Context as _;
use std::sync::atomic::{AtomicUsize, Ordering};

// Frozen from c8829bae0. Keep this independent of the candidate's split helpers.
fn original_read_manifest(
    path: &Path,
    source_id: SourceId,
    ctx: &Context,
) -> anyhow::Result<EitherManifest> {
    let ext = path.extension();
    if ext.is_none() || ext == Some(std::ffi::OsStr::new("rs")) {
        bail!("parsing `{}` requires `-Zscript`", path.display());
    }
    let bytes =
        std::fs::read(path).with_context(|| format!("failed to read `{}`", path.display()))?;
    let contents = String::from_utf8(bytes)
        .map_err(|_| anyhow!("path at `{}` was not valid utf-8", path.display()))?;
    let original_toml: TomlManifest =
        toml::from_str(&contents).map_err(|e| ManifestDiagnostic::from_toml(e, &contents, path))?;

    let mut warnings = Vec::new();
    let mut errors = Vec::new();
    (|| {
        let empty = Vec::new();
        let cargo_features = original_toml.cargo_features.as_ref().unwrap_or(&empty);
        let features = Features::new(cargo_features, &mut warnings, source_id.is_path())?;
        let workspace_config = to_workspace_config(&original_toml, path, &mut warnings)?;
        if let WorkspaceConfig::Root(ws_root_config) = &workspace_config {
            let package_root = path.parent().unwrap();
            ctx.ws_roots.borrow_mut().insert(package_root.to_owned(), (**ws_root_config).clone());
        }
        let normalized_toml = normalize_toml(
            &original_toml,
            &features,
            &workspace_config,
            path,
            ctx,
            &mut warnings,
            &mut errors,
        )?;
        if normalized_toml.package().is_some() {
            to_real_manifest(
                original_toml,
                normalized_toml,
                features,
                workspace_config,
                source_id,
                path,
                ctx,
                &mut warnings,
            )
            .map(|mut m| {
                m.warnings = std::mem::take(&mut warnings);
                EitherManifest::Real(Box::new(m))
            })
        } else if normalized_toml.workspace.is_some() {
            to_virtual_manifest(normalized_toml, features, workspace_config, source_id, path, ctx)
                .map(|mut m| {
                    warnings.append(&mut m.warnings);
                    m.warnings = std::mem::take(&mut warnings);
                    EitherManifest::Virtual(Box::new(m))
                })
        } else {
            bail!("manifest is missing either a `[package]` or a `[workspace]`")
        }
    })()
    .with_context(|| format!("failed to parse manifest at `{}`", path.display()))
}

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "blaze-parallel-manifest-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(path.join("home")).unwrap();
        Self(path)
    }

    fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, bytes).unwrap();
        path
    }

    fn paths(&self) -> Vec<PathBuf> {
        (0..12)
            .map(|i| {
                self.write(&format!("p{i}/src/lib.rs"), "pub fn fixture() {}\n");
                self.write(
                    &format!("p{i}/Cargo.toml"),
                    format!("[package]\nname = 'p{i}'\nversion = '1.0.{i}'\nedition = '2021'\n"),
                )
            })
            .collect()
    }

    fn context(&self) -> Context {
        Context::new(
            Config::load(
                &self.0,
                [("CARGO_HOME".into(), self.0.join("home").display().to_string())],
                &[],
            )
            .unwrap(),
        )
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn source() -> SourceId {
    SourceId::for_registry(&"https://example.invalid/index".parse().unwrap())
}

fn snapshot(result: anyhow::Result<EitherManifest>, cwd: &Path) -> String {
    match result {
        Ok(manifest) => {
            // Full native structures include normalized schema, cfg dependencies,
            // workspace/config fields, targets and warning order, not only JSON.
            let exported = match &manifest {
                EitherManifest::Real(m) => serde_json::to_string(&crate::metadata::export_package(
                    &cwd.join("Cargo.toml"),
                    m,
                ))
                .unwrap(),
                EitherManifest::Virtual(_) => "virtual".into(),
            };
            format!("{manifest:#?}\n{exported}")
        }
        Err(error) => format!(
            "{:?}\n{:?}\n{}",
            error.downcast_ref::<ManifestDiagnostic>(),
            error.chain().map(ToString::to_string).collect::<Vec<_>>(),
            crate::diagnostic::report(&error, cwd),
        ),
    }
}

fn differential(fixture: &Fixture, paths: &[PathBuf]) -> Vec<String> {
    let original = fixture.context();
    let sequential = fixture.context();
    let parallel = fixture.context();
    let parsed = preparse_manifests(paths).expect("regular batch must select parallel syntax");
    let mut observations = Vec::new();
    for (path, syntax) in paths.iter().zip(parsed) {
        let expected = snapshot(original_read_manifest(path, source(), &original), &fixture.0);
        assert_eq!(snapshot(read_manifest(path, source(), &sequential), &fixture.0), expected);
        assert_eq!(snapshot(syntax.read(source(), &parallel), &fixture.0), expected);
        assert_eq!(
            format!("{:?}", original.ws_roots.borrow()),
            format!("{:?}", parallel.ws_roots.borrow())
        );
        let config_warnings = original.config.take_warnings();
        assert_eq!(config_warnings, sequential.config.take_warnings());
        assert_eq!(config_warnings, parallel.config.take_warnings());
        observations.push(expected);
    }
    observations
}

#[test]
fn regular_registry_batch_order_warnings_cfg_and_targets() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fixture.write("p1/Cargo.toml", "[package]\nname='p1'\nversion='1.0.1'\nedition='2018'\n[dev_dependencies]\nhelper='1'\n[target.'cfg(unix)'.dependencies]\nplatform='2'\n");
    fixture.write("p2/src/main.rs", "fn main() {}\n");
    fixture.write("p2/tests/real.rs", "#[test] fn real() {}\n");
    fixture.write("p2/examples/example.rs", "fn main() {}\n");
    let observations = differential(&fixture, &paths);
    assert_eq!(observations.len(), 12);
    assert!(observations[1].contains("dev_dependencies"));
    assert!(observations[1].contains("cfg(unix)"));
    assert!(observations[2].contains("real.rs"));
    assert!(observations[2].contains("example.rs"));
    let EitherManifest::Real(warned) =
        original_read_manifest(&paths[1], source(), &fixture.context()).unwrap()
    else {
        panic!("warning fixture must be a real package");
    };
    assert!(!warned.warnings.is_empty(), "fixture must exercise actual warning preservation");
}

#[test]
fn malformed_utf8_syntax_schema_and_semantic_error_chains() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fixture.write("p0/Cargo.toml", [0xff, 0xfe]);
    fixture.write("p1/Cargo.toml", "[package\nname = 'broken'\n");
    fixture.write("p2/Cargo.toml", "[package]\nname = 42\n");
    fixture.write("p3/Cargo.toml", "[package]\nname = 'p3'\nversion = 'not-a-version'\n");
    fixture.write("p4/Cargo.toml", "[package]\nname='p4'\nversion='1.0.0'\nedition='2099'\n");
    fixture.write("p5/Cargo.toml", "[dependencies]\nmissing='1'\n");
    fixture.write(
        "p6/Cargo.toml",
        "[package]\nname='p6'\nversion='1.0.0'\n[features]\ndefault=['does-not-exist']\n",
    );
    let observations = differential(&fixture, &paths);
    assert!(observations[0].contains("not valid utf-8"));
    assert!(observations[1].contains("p1/Cargo.toml"));
    assert!(observations[2].contains("p2/Cargo.toml"));
    assert!(observations[3].contains("p3/Cargo.toml"));
    assert!(observations[5].contains("failed to parse manifest"));
}

#[test]
fn workspace_inheritance_and_config_stay_sequential() {
    let fixture = Fixture::new();
    let mut paths = fixture.paths();
    let root = fixture.write("Cargo.toml", "[workspace]\nmembers=['p0','p1']\n[workspace.package]\nversion='3.2.1'\nedition='2021'\n[workspace.dependencies]\nhelper='2'\n");
    fixture.write(
        ".cargo/config.toml",
        "[registries.alternate]\nindex='https://alternate.invalid/index'\n",
    );
    fixture.write("p0/Cargo.toml", "[package]\nname='p0'\nversion.workspace=true\nedition.workspace=true\nworkspace='..'\n[dependencies]\nhelper.workspace=true\nother={version='1',registry='alternate'}\n");
    paths.insert(0, root);
    let observations = differential(&fixture, &paths);
    assert!(observations[1].contains("3.2.1"));
    assert!(observations[1].contains("alternate.invalid"));
    // A malformed inheritance root must retain the full original context chain.
    fixture.write("Cargo.toml", "[workspace]\n[workspace.package]\nversion=4\n");
    differential(&fixture, &paths);
}

#[test]
fn repeated_invocations_read_fresh_syntax_semantics_and_target_files() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let before = differential(&fixture, &paths);
    fixture.write(
        "p0/Cargo.toml",
        "[package]\nname='renamed'\nversion='7.6.5'\nedition='2021'\n[dependencies]\nfresh='3'\n",
    );
    fixture.write("p0/src/main.rs", "fn main() {}\n");
    let changed = differential(&fixture, &paths);
    assert_ne!(before[0], changed[0]);
    assert!(changed[0].contains("renamed"));
    assert!(changed[0].contains("7.6.5"));
    assert!(changed[0].contains("main.rs"));
    fixture.write("p0/Cargo.toml", "not valid [toml");
    let broken = differential(&fixture, &paths);
    assert_ne!(changed[0], broken[0]);
    fixture.write("p0/Cargo.toml", "[package]\nname='repaired'\nversion='1.0.0'\n");
    let repaired = differential(&fixture, &paths);
    assert!(repaired[0].contains("repaired"));
}

#[test]
fn unknown_paths_symlinks_directories_and_fifo_are_not_speculatively_read() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    assert!(preparse_manifests(&paths[..7]).is_none());
    let mut unknown = paths.clone();
    unknown[0] = fixture.0.join("missing/Cargo.toml");
    assert!(preparse_manifests(&unknown).is_none());
    unknown[0] = fixture.write("not-a-manifest.rs", "");
    assert!(preparse_manifests(&unknown).is_none());
    let directory = fixture.0.join("directory/Cargo.toml");
    std::fs::create_dir_all(&directory).unwrap();
    unknown[0] = directory;
    assert!(preparse_manifests(&unknown).is_none());
    let link = fixture.0.join("link/Cargo.toml");
    std::fs::create_dir(link.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&paths[0], &link).unwrap();
    unknown[0] = link;
    assert!(preparse_manifests(&unknown).is_none());
    // mkfifo is a fixture setup command only. The parser must return without
    // opening this FIFO, with no writer process and no opportunity to block.
    let fifo = fixture.0.join("fifo/Cargo.toml");
    std::fs::create_dir(fifo.parent().unwrap()).unwrap();
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().unwrap().success());
    unknown[0] = fifo;
    assert!(preparse_manifests(&unknown).is_none());
}

#[test]
fn public_sequential_path_extensions_missing_files_and_virtual_manifest() {
    let fixture = Fixture::new();
    let ctx = fixture.context();
    for path in [
        fixture.write("script.rs", "fn main() {}"),
        fixture.write("no-extension", ""),
        fixture.0.join("missing.toml"),
        fixture.write("Cargo.toml", "[workspace]\nmembers=[]\n"),
    ] {
        let original = fixture.context();
        assert_eq!(
            snapshot(read_manifest(&path, source(), &ctx), &fixture.0),
            snapshot(original_read_manifest(&path, source(), &original), &fixture.0),
        );
    }
}
