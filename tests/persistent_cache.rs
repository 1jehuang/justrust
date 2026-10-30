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
            assert!(hit.cache.native_objects.hits > 0, "{:?}", hit.cache);
            assert_eq!(hit.cache.native_objects.misses, 0, "{:?}", hit.cache);
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
fn optimized_facts_and_machine_code_reuse_across_body_and_signature_edits() {
    use compiler::ssa::OptMode;
    let scratch = Scratch::new();
    let original = "fn stable(x:i32)->i32 { let mut y=x; let mut i=0; while i<20 { y=(y*3+1)%97; i+=1; } y }\nfn changing(x:i32)->i32 { x+7 }\nfn main() { println!(\"{}\", stable(4)+changing(2)); }";
    for workers in [1, 8] {
        let path = scratch.0.join(format!("optimized-{workers}"));
        let build = |source: &str, cached: bool| {
            let mut config = options(&path);
            config.opt = OptMode::Final;
            if !cached {
                config.local_cache = None;
            }
            compiler::compile_observed_with_options(source.into(), workers, false, config)
                .unwrap_or_else(|error| panic!("{error}"))
        };
        let cold = build(original, true);
        assert!(cold.cache.ssa_lowering.stored > 0, "{:?}", cold.cache);
        assert!(cold.cache.local_optimization.stored > 0, "{:?}", cold.cache);
        assert!(cold.cache.ssa_objects.stored > 0, "{:?}", cold.cache);
        let warm = build(original, true);
        assert!(warm.cache.ssa_lowering.hits > 0, "{:?}", warm.cache);
        assert_eq!(warm.cache.ssa_lowering.misses, 0, "{:?}", warm.cache);
        assert!(warm.cache.local_optimization.hits > 0, "{:?}", warm.cache);
        assert!(warm.cache.ssa_objects.hits > 0, "{:?}", warm.cache);
        assert_eq!(warm.cache.local_optimization.misses, 0, "{:?}", warm.cache);
        assert_eq!(warm.cache.ssa_objects.misses, 0, "{:?}", warm.cache);
        assert_eq!(warm.executable, cold.executable);
        for (revision, source) in [
            original.to_owned(),
            original.replace("x+7", "x+8"),
            original
                .replace("changing(x:i32)", "changing(x:i32,y:i32)")
                .replace("x+7", "x+y")
                .replace("changing(2)", "changing(2,7)"),
        ]
        .into_iter()
        .enumerate()
        {
            let cached = build(&source, true);
            let clean = build(&source, false);
            assert_eq!(cached.executable, clean.executable);
            assert_eq!(cached.report.nodes, clean.report.nodes);
            assert_eq!(cached.report.edges, clean.report.edges);
            assert!(
                cached.cache.local_optimization.hits > 0,
                "unrelated body lost reuse: {:?}",
                cached.cache
            );
            assert!(cached.cache.ssa_lowering.hits > 0, "{:?}", cached.cache);
            if revision > 0 {
                assert!(cached.cache.ssa_lowering.misses > 0, "{:?}", cached.cache);
                assert!(
                    cached.cache.local_optimization.misses > 0,
                    "changed body was reused: {:?}",
                    cached.cache
                );
            }
            let native = execute(
                &scratch
                    .0
                    .join(format!("optimized-native-{workers}-{revision}")),
                cached.executable.as_ref().unwrap(),
            );
            let input = scratch.0.join("optimized.rs");
            let stock_path = scratch.0.join("optimized-stock");
            fs::write(&input, &source).unwrap();
            let stock = Command::new("rustc")
                .arg(&input)
                .args(["--edition=2024", "-Cpanic=abort", "-Copt-level=2", "-o"])
                .arg(&stock_path)
                .output()
                .unwrap();
            assert!(
                stock.status.success(),
                "{}",
                String::from_utf8_lossy(&stock.stderr)
            );
            let stock = Command::new(&stock_path).output().unwrap();
            assert_eq!(native.status.code(), stock.status.code());
            assert_eq!(native.stdout, stock.stdout);
            assert_eq!(native.stderr, stock.stderr);
        }
        for entry in fs::read_dir(&path).unwrap() {
            fs::write(entry.unwrap().path(), b"corrupt").unwrap();
        }
        let repaired = build(original, true);
        assert_eq!(repaired.executable, cold.executable);
        assert!(repaired.cache.local_optimization.misses > 0);
        let invalid = format!("{original}\nfn unused()->i32 {{ true }}");
        let mut config = options(&path);
        config.opt = OptMode::Final;
        assert!(
            compiler::compile_observed_with_options(invalid, workers, false, config.clone())
                .is_err()
        );
        let checked =
            compiler::compile_observed_with_options(original.into(), workers, true, config)
                .unwrap_or_else(|error| panic!("{error}"));
        assert_eq!(checked.cache, compiler::cache::Report::default());
        assert!(!checked.report.published_kind(103));
    }
}
#[test]
fn structural_native_outputs_and_overflow_profiles_match_stock() {
    let scratch = Scratch::new();
    let fixtures = [
        "fn value(x:i32)->i32 { let a=[x,7,11]; let pair=(a[1],a[2]); let r=&a[0]; *r+pair.0+pair.1 } fn main(){ println!(\"{}\",value(3)); }",
        "fn value(x:f64)->f64 { (x*2.0+1.0)/3.0 } fn main(){ if value(4.0)==3.0 { println!(\"float\"); } }",
        "fn value(x:u8)->u8 { x+1 } fn main(){ println!(\"{}\",value(254)); }",
    ];
    for (fixture, source) in fixtures.into_iter().enumerate() {
        let input = scratch.0.join("structural.rs");
        fs::write(&input, source).unwrap();
        for checked in [false, true] {
            let stock_path = scratch.0.join("structural-stock");
            let stock = Command::new("rustc")
                .arg(&input)
                .args(["--edition=2024", "-Cpanic=abort", "-C"])
                .arg(format!("overflow-checks={checked}"))
                .arg("-o")
                .arg(&stock_path)
                .output()
                .unwrap();
            assert!(
                stock.status.success(),
                "{}",
                String::from_utf8_lossy(&stock.stderr)
            );
            let expected = Command::new(&stock_path).output().unwrap();
            let cache = scratch.0.join(format!("structural-{fixture}"));
            for workers in [1, 8] {
                let mut config = options(&cache);
                config.overflow_checks = checked;
                let cold = compiler::compile_observed_with_options(
                    source.into(),
                    workers,
                    false,
                    config.clone(),
                )
                .unwrap_or_else(|e| panic!("{e}"));
                let warm = compiler::compile_observed_with_options(
                    source.into(),
                    workers,
                    false,
                    config.clone(),
                )
                .unwrap_or_else(|e| panic!("{e}"));
                assert!(warm.cache.native_objects.hits > 0, "{:?}", warm.cache);
                assert_eq!(warm.cache.native_objects.misses, 0, "{:?}", warm.cache);
                config.local_cache = None;
                let clean =
                    compiler::compile_observed_with_options(source.into(), workers, false, config)
                        .unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(warm.executable, clean.executable);
                assert_eq!(cold.executable, clean.executable);
                let actual = execute(
                    &scratch.0.join("structural-native"),
                    warm.executable.as_ref().unwrap(),
                );
                assert_eq!(actual.status.code(), expected.status.code());
                assert_eq!(actual.stdout, expected.stdout);
                assert_eq!(actual.stderr, expected.stderr);
            }
        }
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
fn computed_export_cli_rejects_existing_directory_and_invalid_unused_source() {
    let scratch = Scratch::new();
    let source_root = scratch.0.join("src");
    let cache = scratch.0.join("cache");
    let export = scratch.0.join("export");
    fs::create_dir(&source_root).unwrap();
    fs::create_dir(&cache).unwrap();
    fs::create_dir(&export).unwrap();
    fs::write(export.join("sentinel"), b"preserve").unwrap();
    let source = source_root.join("main.rs");
    fs::write(&source, "fn main() {}").unwrap();
    let invoke = |export: &std::path::Path, output: &str| {
        Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .arg("-o")
            .arg(scratch.0.join(output))
            .arg("--cache-dir")
            .arg(&cache)
            .arg("--computed-operation-export")
            .arg(export)
            .output()
            .unwrap()
    };
    let result = invoke(&export, "existing-out");
    assert!(!result.status.success());
    assert_eq!(fs::read(export.join("sentinel")).unwrap(), b"preserve");
    assert!(!export.join("manifest.json").exists());
    let cold = invoke(&scratch.0.join("cold"), "cold-out");
    assert!(
        cold.status.success(),
        "{}",
        String::from_utf8_lossy(&cold.stderr)
    );
    fs::write(&source, "fn unused() { let x: bool = 1; } fn main() {}").unwrap();
    let bad_export = scratch.0.join("bad");
    let bad = invoke(&bad_export, "bad-out");
    assert!(!bad.status.success());
    assert!(!bad_export.join("manifest.json").exists());
    assert!(!scratch.0.join("bad-out").exists());
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
        let export = scratch.0.join(format!("export-{run}"));
        let result = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .arg("-o")
            .arg(&output)
            .arg("--cache-dir")
            .arg(&cache)
            .arg("--computed-operation-export")
            .arg(&export)
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
        let manifest = fs::read_to_string(export.join("manifest.json")).unwrap();
        assert!(manifest.len() < 1024);
        if run == 0 {
            let receipt = fs::read(export.join("computed-native-link-v1")).unwrap();
            assert_eq!(receipt.len(), 104);
            let key = blake3::Hash::from(<[u8; 32]>::try_from(&receipt[32..64]).unwrap())
                .to_hex()
                .to_string();
            let payload = fs::read(export.join(&key)).unwrap();
            assert_eq!(payload, fs::read(&output).unwrap());
            assert_eq!(blake3::hash(&payload).as_bytes(), &receipt[64..96]);
            assert!(manifest.contains(&format!("\"operation_key\":\"{key}\"")));
            assert!(manifest.contains(&format!("\"payload_path\":\"{key}\"")));
            assert!(manifest.contains("\"operation_version\":1"));
            assert!(manifest.contains("\"operation\":\"justrust/native-link/x86_64-linux\""));
            assert_ne!(&receipt[..32], &[0; 32]);
            assert_eq!(fs::read_dir(&export).unwrap().count(), 3);
        } else {
            assert_eq!(
                manifest,
                "{\"format\":\"justrust-computed-operations\",\"hash\":\"blake3-256\",\"operations\":[],\"version\":1}\n"
            );
            assert_eq!(fs::read_dir(&export).unwrap().count(), 1);
        }
    }
}

#[test]
fn exact_source_objects_skip_optimizer_but_not_checks() {
    use compiler::{source::SourceSnapshot, ssa::OptMode};
    let scratch = Scratch::new();
    let root = scratch.0.join("inputs");
    fs::create_dir(&root).unwrap();
    let entry = root.join("main.rs");
    let text = "fn identity<T>(x:T)->T {x} fn twice(x:i32)->i32 {identity(x*2)} fn main(){ println!(\"{}\",twice(21)); }";
    let stock_output = |text: &str| {
        fs::write(&entry, text).unwrap();
        let executable = scratch.0.join("exact-stock");
        let result = Command::new("rustc")
            .arg(&entry)
            .args(["--edition=2024", "-Cpanic=abort", "-o"])
            .arg(&executable)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        Command::new(executable).output().unwrap()
    };
    let stock = stock_output(text);
    for workers in [1, 8] {
        let directory = scratch.0.join(format!("exact-{workers}"));
        let build = |text: &str, sibling: Option<&str>, cached: bool, check: bool| {
            let mut config = options(&directory);
            config.opt = OptMode::Final;
            let mut files = vec![(PathBuf::from("main.rs"), text.as_bytes().to_vec())];
            if let Some(sibling) = sibling {
                files.push((PathBuf::from("optional.rs"), sibling.as_bytes().to_vec()));
            }
            config.source_snapshot = Some(SourceSnapshot::new(root.clone(), files).unwrap());
            if !cached {
                config.local_cache = None;
            }
            compiler::compile_file_observed_with_options(entry.clone(), workers, check, config)
        };
        let clean = build(text, None, false, false).unwrap();
        let cold = build(text, None, true, false).unwrap();
        assert!(
            cold.cache.exact_input_objects.stored > 0,
            "{:?}",
            cold.cache
        );
        assert!(cold.report.published_kind(133));
        let warm = build(text, None, true, false).unwrap();
        assert!(warm.cache.exact_input_objects.hits > 0, "{:?}", warm.cache);
        assert_eq!(warm.cache.exact_input_objects.misses, 0);
        assert!(!warm.report.published_kind(130), "hit demanded SSA");
        assert!(!warm.report.published_kind(133), "hit demanded IPO_OPT");
        for kind in [104, 109, 126, 128, 142] {
            assert!(
                warm.report.published_kind(kind),
                "hit omitted proof kind {kind}"
            );
        }
        assert_eq!(warm.executable, clean.executable);
        assert_eq!(cold.executable, clean.executable);
        let actual = execute(
            &scratch.0.join(format!("exact-run-{workers}")),
            warm.executable.as_ref().unwrap(),
        );
        assert_eq!(actual.status.code(), stock.status.code());
        assert_eq!(actual.stdout, stock.stdout);
        assert_eq!(actual.stderr, stock.stderr);
        // Even an unused file's absence is part of this conservative identity.
        let changed_namespace = build(text, Some("fn helper() {}"), true, false).unwrap();
        assert_eq!(changed_namespace.cache.exact_input_objects.hits, 0);
        assert!(changed_namespace.cache.exact_input_objects.stored > 0);
        let edited = text.replace("x*2", "x*3");
        let changed_body = build(&edited, None, true, false).unwrap();
        assert_eq!(changed_body.cache.exact_input_objects.hits, 0);
        assert_eq!(
            changed_body.executable,
            build(&edited, None, false, false).unwrap().executable
        );
        let invalid = format!("{text} fn unused(){{ let x:bool=1; }}");
        assert!(build(&invalid, None, true, false).is_err());
        let check = build(text, None, true, true).unwrap();
        assert!(check.executable.is_none());
        assert!(!check.report.published_kind(103));
        for entry in fs::read_dir(&directory).unwrap() {
            fs::write(entry.unwrap().path(), b"corrupt").unwrap();
        }
        let repaired = build(text, None, true, false).unwrap();
        assert_eq!(repaired.cache.exact_input_objects.hits, 0);
        assert!(repaired.cache.exact_input_objects.stored > 0);
        assert_eq!(repaired.executable, clean.executable);
        let nominal = "struct Pair { x:i32,y:i32 } fn total(p:Pair)->i32 {p.x+p.y} fn main(){println!(\"{}\",total(Pair{x:20,y:22}));}";
        let nominal_cold = build(nominal, None, true, false).unwrap();
        assert!(nominal_cold.cache.exact_input_objects.stored > 0);
        let nominal_warm = build(nominal, None, true, false).unwrap();
        assert!(nominal_warm.cache.exact_input_objects.hits > 0);
        assert_eq!(
            nominal_warm.executable,
            build(nominal, None, false, false).unwrap().executable
        );
        let expected = stock_output(nominal);
        let actual = execute(
            &scratch.0.join(format!("nominal-run-{workers}")),
            nominal_warm.executable.as_ref().unwrap(),
        );
        assert_eq!(actual.status.code(), expected.status.code());
        assert_eq!(actual.stdout, expected.stdout);
        assert_eq!(actual.stderr, expected.stderr);
    }
}
