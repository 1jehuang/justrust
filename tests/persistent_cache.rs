//! Cross-compilation native output reuse, not semantic-proof restoration.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::{
    compiler::{self, link_cache::Outcome, source::SourceOptions},
    persistent_cache::LocalCache,
};
use std::os::unix::fs::PermissionsExt;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let path = root.join(format!(
            "jr-persistent-cache-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn options(path: &Path) -> SourceOptions {
    SourceOptions {
        prelude: false,
        local_cache: Some(LocalCache::new(path.to_owned())),
        ..SourceOptions::default()
    }
}
fn compile(source: &str, workers: usize, path: &Path) -> compiler::Compilation {
    compiler::compile_observed_with_options(source.into(), workers, false, options(path))
        .unwrap_or_else(|e| panic!("{e}"))
}
fn execute(path: &Path, bytes: &[u8]) -> std::process::Output {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    for attempt in 0..100 {
        match Command::new(path).output() {
            Err(error) if error.raw_os_error() == Some(26) && attempt < 99 => {
                std::thread::sleep(std::time::Duration::from_millis(1))
            }
            output => return output.unwrap(),
        }
    }
    unreachable!()
}
#[test]
fn repeated_builds_edits_and_corruption_match_clean_and_stock() {
    let scratch = Scratch::new();
    for workers in [1, 8] {
        let cache = scratch.0.join(format!("cache-{workers}"));
        for (revision, text) in ["first", "second"].into_iter().enumerate() {
            let source = format!("fn leaf() {{ println!(\"{text}\"); }} fn main() {{ leaf(); }}");
            let clean = compiler::compile_observed_with_options(
                source.clone(),
                workers,
                false,
                SourceOptions {
                    prelude: false,
                    ..SourceOptions::default()
                },
            )
            .unwrap_or_else(|e| panic!("{e}"));
            let miss = compile(&source, workers, &cache);
            assert_eq!(miss.link_cache, Outcome::MissStored);
            let hit = compile(&source, workers, &cache);
            assert_eq!(hit.link_cache, Outcome::Hit);
            assert_eq!(hit.executable, clean.executable);
            assert_eq!(miss.executable, clean.executable);
            assert_eq!(hit.report.nodes, clean.report.nodes);
            assert_eq!(hit.report.edges, clean.report.edges);
            let native = execute(
                &scratch.0.join(format!("native-{workers}-{revision}")),
                hit.executable.as_ref().unwrap(),
            );
            let input = scratch.0.join("input.rs");
            let stock = scratch.0.join("stock");
            fs::write(&input, &source).unwrap();
            let built = Command::new("rustc")
                .arg(&input)
                .args(["--edition=2024", "-Cpanic=abort", "-o"])
                .arg(&stock)
                .output()
                .unwrap();
            assert!(
                built.status.success(),
                "{}",
                String::from_utf8_lossy(&built.stderr)
            );
            let stock = Command::new(&stock).output().unwrap();
            assert_eq!(native.status.code(), stock.status.code());
            assert_eq!(native.stdout, stock.stdout);
            assert_eq!(native.stderr, stock.stderr);
        }
        // A corrupted record is a miss, never a successful restored executable.
        for entry in fs::read_dir(&cache).unwrap() {
            fs::write(entry.unwrap().path(), b"truncated").unwrap();
        }
        let source = "fn leaf() { println!(\"second\"); } fn main() { leaf(); }";
        let repaired = compile(source, workers, &cache);
        assert_eq!(repaired.link_cache, Outcome::MissStored);
        assert_eq!(compile(source, workers, &cache).link_cache, Outcome::Hit);
        // A warm link result cannot hide an invalid unused item.
        let invalid = format!("{source} fn unused() -> i32 {{ true }}");
        assert!(
            compiler::compile_observed_with_options(invalid, workers, false, options(&cache))
                .is_err()
        );
        let unsupported = format!("{source} async fn unused_async() {{}}");
        assert!(
            compiler::compile_observed_with_options(unsupported, workers, false, options(&cache))
                .is_err()
        );
        let signature_edit =
            "fn leaf(x: i32) { if x == 7 { println!(\"second\"); } } fn main() { leaf(7); }";
        let edited = compile(signature_edit, workers, &cache);
        assert_eq!(edited.link_cache, Outcome::MissStored);
        assert_eq!(
            compile(signature_edit, workers, &cache).link_cache,
            Outcome::Hit
        );
        let clean = compiler::compile_observed_with_options(
            signature_edit.into(),
            workers,
            false,
            SourceOptions {
                prelude: false,
                ..SourceOptions::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(edited.executable, clean.executable);
        let checked =
            compiler::compile_observed_with_options(source.into(), workers, true, options(&cache))
                .unwrap_or_else(|e| panic!("{e}"));
        assert!(checked.executable.is_none());
        assert_eq!(checked.link_cache, Outcome::Disabled);
        assert!(!checked.report.published_kind(105), "CHECK demanded BUILD");
        assert!(!checked.report.published_kind(103), "CHECK demanded OBJECT");
    }
}
#[test]
fn unwritable_cache_fails_open_and_configuration_invalidates() {
    let scratch = Scratch::new();
    let blocked = scratch.0.join("not-a-directory");
    fs::write(&blocked, b"keep").unwrap();
    let source = "fn main() {}";
    assert_eq!(
        compile(source, 1, &blocked).link_cache,
        Outcome::MissNotStored
    );
    assert_eq!(fs::read(&blocked).unwrap(), b"keep");
    let cache = scratch.0.join("cache");
    assert_eq!(compile(source, 1, &cache).link_cache, Outcome::MissStored);
    let mut unwind = options(&cache);
    unwind.panic_unwind = true;
    let result = compiler::compile_observed_with_options(source.into(), 8, false, unwind.clone())
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(result.link_cache, Outcome::MissStored);
    let hit = compiler::compile_observed_with_options(source.into(), 1, false, unwind)
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(hit.link_cache, Outcome::Hit);
    assert_eq!(result.executable, hit.executable);
}
#[test]
fn standalone_processes_reuse_outputs_despite_distinct_output_arguments() {
    let scratch = Scratch::new();
    let source_root = scratch.0.join("src");
    let cache = scratch.0.join("cache");
    fs::create_dir(&source_root).unwrap();
    fs::create_dir(&cache).unwrap();
    let source = source_root.join("main.rs");
    fs::write(&source, "fn main() { println!(\"cached\"); }").unwrap();
    for (run, expected) in ["MissStored", "Hit"].into_iter().enumerate() {
        let output = scratch.0.join(format!("out-{run}"));
        let result = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .arg("--cache-dir")
            .arg(&cache)
            .args(["--verbose", "--workers", if run == 0 { "1" } else { "8" }])
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(
            String::from_utf8_lossy(&result.stderr)
                .contains(&format!("Native link cache: {expected}")),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(Command::new(&output).output().unwrap().stdout, b"cached\n");
    }
}
