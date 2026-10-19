//! Runtime lane: drop glue, drop elaboration, process exit, panics.
//! Fixtures under tests/fixtures/runtime carry stock rustc (edition 2024,
//! -C panic=abort) stdout and exit status; reject_* fixtures are rejected by stock.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::path::PathBuf;
use std::process::Command;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/runtime")
}

fn scratch(name: &str, workers: u32) -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!(
        "jr-runtime-{}-{name}-{workers}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn runtime_fixtures_match_stock() {
    let mut names: Vec<String> = std::fs::read_dir(fixtures())
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            // pending_* fixtures wait on a feature owned by another lane.
            name.strip_suffix(".rs")
                .filter(|n| !n.starts_with("pending_"))
                .map(str::to_owned)
        })
        .collect();
    names.sort();
    assert!(names.len() >= 5);
    for name in &names {
        let source = fixtures().join(format!("{name}.rs"));
        for workers in [1, 8] {
            let dir = scratch(name, workers);
            let exe = dir.join("native");
            let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&source)
                .arg("-o")
                .arg(&exe)
                .args(["--workers", &workers.to_string()])
                .output()
                .unwrap();
            if name.starts_with("reject_") {
                assert_eq!(out.status.code(), Some(1), "{name}: expected rejection");
                assert!(!exe.exists(), "{name}: rejected program produced output");
                continue;
            }
            assert!(
                out.status.success(),
                "{name}: build failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let run = Command::new(&exe).output().unwrap();
            let want = std::fs::read(fixtures().join(format!("{name}.stdout"))).unwrap();
            let status: i32 = std::fs::read_to_string(fixtures().join(format!("{name}.status")))
                .unwrap()
                .trim()
                .parse()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&want),
                "{name}: stdout"
            );
            assert_eq!(run.status.code(), Some(status), "{name}: exit status");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
