//@ needs-target-std
//@ only-x86_64-unknown-linux-gnu
//@ ignore-backends: gcc
//
// Losing one cached object must rebuild that product, not discard all
// independent CGUs and frontend queries. Keep compiler bytes and cache dirs
// isolated: this fixture only uses the compiler selected by compiletest.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use run_make_support::{run, rustc};

fn build(cache: &str, state: &str, opt: &str, lto: &str) -> BTreeMap<String, String> {
    let output = rustc()
        .input("input.rs")
        .crate_name("example")
        .output("example")
        .incremental(cache)
        .codegen_units(8)
        .opt_level(opt)
        .lto(lto)
        .arg("-Cembed-bitcode=yes")
        .arg("-Cdebuginfo=2")
        .arg("-Csplit-debuginfo=unpacked")
        .arg("-Zsplit-dwarf-kind=split")
        .arg("-Zquery-dep-graph")
        .arg("-Zprint-mono-items")
        .arg(format!("-Zassert-incr-state={state}"))
        .run()
        .stdout_utf8();
    output
        .lines()
        .filter_map(|line| {
            let mut fields = line.strip_prefix("CGU_REUSE ")?.split_whitespace();
            Some((fields.next()?.to_owned(), fields.next()?.to_owned()))
        })
        .collect()
}

fn objects(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            found.extend(objects(&path));
        } else if path.extension().is_some_and(|ext| ext == "o") {
            found.push(path);
        }
    }
    found.sort();
    found
}

fn exercise(lto: &str) {
    let source = std::fs::read_to_string("main.rs").unwrap();
    std::fs::write("input.rs", &source).unwrap();
    let cache = format!("incr-{lto}");
    build(&cache, "not-loaded", "0", lto);
    run("example").assert_stdout_equals("42");

    let warm = build(&cache, "loaded", "0", lto);
    let reused = if lto == "off" { "PostLto" } else { "PreLto" };
    let mut candidates: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for path in objects(Path::new(&cache)) {
        let cgu = path.file_stem().unwrap().to_str().unwrap();
        if warm.get(cgu).is_some_and(|v| v == reused) {
            candidates.entry(cgu.to_owned()).or_default().push(path);
        }
    }
    assert!(candidates.len() >= 2, "need independent reused CGUs: {warm:?}");
    let mut candidates = candidates.into_iter();
    let (missing_cgu, missing_files) = candidates.next().unwrap();
    let (retained_cgu, _) = candidates.next().unwrap();
    // GC may retain older session directories. Remove this CGU's object
    // from all of them so the assertion cannot find an obsolete copy.
    for missing in missing_files {
        assert!(missing.with_extension("dwo").exists(), "expected partial multi-file product");
        std::fs::remove_file(missing).unwrap();
    }

    // This assertion fails before the fix: losing any product discarded
    // the whole depgraph, so even -Zassert-incr-state=loaded failed.
    let repaired = build(&cache, "loaded", "0", lto);
    assert_eq!(repaired.get(&missing_cgu).map(String::as_str), Some("No"));
    assert_eq!(repaired.get(&retained_cgu).map(String::as_str), Some(reused));
    assert!(
        objects(Path::new(&cache))
            .iter()
            .any(|path| { path.file_stem().unwrap().to_str().unwrap() == missing_cgu }),
        "missing product was not regenerated"
    );
    run("example").assert_stdout_equals("42");

    // Keeping the graph must not bypass source-dependent invalidation.
    assert!(source.contains("40"));
    std::fs::write("input.rs", source.replace("40", "41")).unwrap();
    let changed = build(&cache, "loaded", "0", lto);
    assert!(changed.values().any(|reuse| reuse == "No"));
    run("example").assert_stdout_equals("43");

    // Nor may it bypass the existing tracked compiler-argument hash gate.
    build(&cache, "not-loaded", "1", lto);
    run("example").assert_stdout_equals("43");
}

fn main() {
    exercise("off");
    exercise("thin");
}
