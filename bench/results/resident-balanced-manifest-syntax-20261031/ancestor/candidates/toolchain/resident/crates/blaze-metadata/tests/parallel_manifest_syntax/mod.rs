//! Real-filesystem fixtures for the fetched registry fast path. The sequential
//! oracle is the accepted fetch loop, with fetching replaced only by local roots.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "blaze-metadata-parallel-manifest-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::create_dir(path.join("home")).unwrap();
        Self(path)
    }

    fn write(&self, relative: &str, text: impl AsRef<[u8]>) -> PathBuf {
        let path = self.0.join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn paths(&self) -> Vec<PathBuf> {
        (0..12)
            .map(|i| {
                self.write(&format!("p{i}/src/lib.rs"), "pub fn fixture() {}\n");
                self.write(
                    &format!("p{i}/Cargo.toml"),
                    format!("[package]\nname='p{i}'\nversion='1.0.{i}'\nedition='2021'\n"),
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

    fn texts(&self, len: usize) -> Vec<String> {
        (0..len).map(|_| "registry+https://example.invalid/index".into()).collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// The original accepted ordered fetch tail. No candidate fast-path helpers here.
fn original_read_fetched(
    texts: Vec<String>,
    roots: Vec<PathBuf>,
    ctx: &Context,
) -> Result<Vec<Package>> {
    let mut read = Vec::with_capacity(texts.len());
    for (source_text, dir) in texts.into_iter().zip(roots) {
        let manifest_path = dir.join("Cargo.toml");
        let source = manifest_source(&source_text, &ctx.config)?;
        let EitherManifest::Real(manifest) = read_manifest(&manifest_path, source, ctx)? else {
            bail!("{} has no package", manifest_path.display());
        };
        read.push(Package { manifest_path, manifest, locked_source: Some(source_text) });
    }
    Ok(read)
}

fn snapshot(result: Result<Vec<Package>>, root: &Path) -> String {
    match result {
        Ok(packages) => packages
            .into_iter()
            .map(|p| {
                format!(
                    "{}\n{:?}\n{:#?}\n{}",
                    p.manifest_path.display(),
                    p.locked_source,
                    p.manifest,
                    serde_json::to_string(&export_package(&p.manifest_path, &p.manifest)).unwrap(),
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Err(error) => format!(
            "{:?}\n{}",
            error.chain().map(ToString::to_string).collect::<Vec<_>>(),
            blaze_manifest::diagnostic::report(&error, root),
        ),
    }
}

fn differential(fixture: &Fixture, texts: &[String], paths: &[PathBuf]) -> String {
    let original_ctx = fixture.context();
    let parallel_ctx = fixture.context();
    let roots = paths.iter().map(|p| p.parent().unwrap().to_owned()).collect();
    let expected =
        snapshot(original_read_fetched(texts.to_vec(), roots, &original_ctx), &fixture.0);
    let actual = read_fetched_parallel(texts, paths, &parallel_ctx, true)
        .map(|value| value.expect("eligible fixture batch must select parallel syntax"));
    assert_eq!(snapshot(actual, &fixture.0), expected);
    assert_eq!(original_ctx.config.take_warnings(), parallel_ctx.config.take_warnings());
    expected
}

#[test]
fn vector_order_native_manifest_fields_warnings_and_targets() {
    let fixture = Fixture::new();
    let mut paths = fixture.paths();
    fixture.write("p1/Cargo.toml", "[package]\nname='p1'\nversion='1.0.1'\nedition='2018'\n[dev_dependencies]\nhelper='1'\n[target.'cfg(unix)'.dependencies]\nplatform='2'\n");
    fixture.write("p2/tests/real.rs", "#[test] fn real() {}\n");
    paths.swap(0, 8);
    paths.swap(2, 11);
    let output = differential(&fixture, &fixture.texts(paths.len()), &paths);
    assert!(output.starts_with(&paths[0].display().to_string()));
    assert!(output.contains("real.rs"));
    assert!(output.contains("cfg(unix)"));
}

#[test]
fn multiple_syntax_errors_report_first_input_error_not_first_worker() {
    let fixture = Fixture::new();
    let mut paths = fixture.paths();
    fixture.write("p1/Cargo.toml", "[package\nfirst broken");
    fixture.write("p8/Cargo.toml", [0xff]);
    let texts = fixture.texts(paths.len());
    let output = differential(&fixture, &texts, &paths);
    assert!(output.contains("p1/Cargo.toml"));
    assert!(!output.contains("p8/Cargo.toml"));
    paths.swap(1, 8);
    let output = differential(&fixture, &texts, &paths);
    assert!(output.contains("p8/Cargo.toml"));
    assert!(!output.contains("p1/Cargo.toml"));
}

#[test]
fn earlier_semantic_error_precedes_later_syntax_error_and_virtual_error() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let texts = fixture.texts(paths.len());
    fixture.write(
        "p0/Cargo.toml",
        "[package]\nname='p0'\nversion='1.0.0'\n[features]\ndefault=['missing-feature']\n",
    );
    fixture.write("p1/Cargo.toml", "[package\nbroken");
    let output = differential(&fixture, &texts, &paths);
    assert!(output.contains("p0/Cargo.toml"));
    assert!(!output.contains("p1/Cargo.toml"));
    fixture.write("p0/Cargo.toml", "[workspace]\nmembers=[]\n");
    let output = differential(&fixture, &texts, &paths);
    assert!(output.contains("p0/Cargo.toml has no package"));
    assert!(!output.contains("p1/Cargo.toml"));
}

#[test]
fn registry_config_error_precedes_same_manifest_syntax_error() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    fixture.write(".cargo/config.toml", "[registry]\nindex='https://forbidden.invalid/index'\n");
    fixture.write("p0/Cargo.toml", [0xff]);
    let texts = (0..paths.len())
        .map(|_| format!("registry+{}", blaze_manifest::source::CRATES_IO_INDEX))
        .collect::<Vec<_>>();
    let output = differential(&fixture, &texts, &paths);
    assert!(output.contains("registry.index"));
    assert!(!output.contains("not valid utf-8"));
}

#[test]
fn disabled_tiny_unknown_git_and_nonregular_batches_decline() {
    let fixture = Fixture::new();
    let mut paths = fixture.paths();
    let texts = fixture.texts(paths.len());
    let ctx = fixture.context();
    assert!(read_fetched_parallel(&texts, &paths, &ctx, false).unwrap().is_none());
    assert!(read_fetched_parallel(&texts[..7], &paths[..7], &ctx, true).unwrap().is_none());
    assert!(read_fetched_parallel(&texts[..7], &paths, &ctx, true).unwrap().is_none());
    let mut unknown = texts.clone();
    unknown[2] = "git+https://example.invalid/repo".into();
    assert!(read_fetched_parallel(&unknown, &paths, &ctx, true).unwrap().is_none());
    unknown[2] = "unknown+https://example.invalid/repo".into();
    assert!(read_fetched_parallel(&unknown, &paths, &ctx, true).unwrap().is_none());
    paths[0] = fixture.0.join("missing/Cargo.toml");
    assert!(read_fetched_parallel(&texts, &paths, &ctx, true).unwrap().is_none());
}

#[test]
fn fresh_reads_preserve_workspace_inheritance_config_and_target_edits() {
    let fixture = Fixture::new();
    let paths = fixture.paths();
    let texts = fixture.texts(paths.len());
    fixture
        .write("Cargo.toml", "[workspace]\nmembers=['p0']\n[workspace.package]\nversion='2.0.0'\n");
    fixture
        .write("p0/Cargo.toml", "[package]\nname='p0'\nversion.workspace=true\nworkspace='..'\n");
    let before = differential(&fixture, &texts, &paths);
    assert!(before.contains("2.0.0"));
    fixture
        .write("Cargo.toml", "[workspace]\nmembers=['p0']\n[workspace.package]\nversion='3.0.0'\n");
    fixture.write("p0/src/main.rs", "fn main() {}\n");
    let after = differential(&fixture, &texts, &paths);
    assert!(after.contains("3.0.0"));
    assert!(after.contains("main.rs"));
    assert_ne!(before, after);
    fixture.write("p0/Cargo.toml", "[package]\nname='p0'\nversion='4.0.0'\n[dependencies]\nother={version='1',registry='alternate'}\n");
    fixture
        .write(".cargo/config.toml", "[registries.alternate]\nindex='https://one.invalid/index'\n");
    let first = differential(&fixture, &texts, &paths);
    fixture
        .write(".cargo/config.toml", "[registries.alternate]\nindex='https://two.invalid/index'\n");
    let second = differential(&fixture, &texts, &paths);
    assert!(first.contains("one.invalid"));
    assert!(second.contains("two.invalid"));
    assert_ne!(first, second);
}
