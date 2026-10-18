//! Trait objects and `impl Trait` (TRAIT-OBJECTS.md). `fixtures/trait_objects/
//! stock.tsv` and the `.stdout` files were recorded from stock rustc first
//! (`scripts/trait_objects_oracle.sh`). Native CHECK and BUILD run through the
//! public `jr-rust` CLI at workers 1 and 8:
//! - positive: stock accepts; native must accept and match stdout exactly.
//! - negative: stock rejects; native must reject (even unused code).
//! - boundary: stock accepts; native must fail with an explicit `unsupported`
//!   diagnostic and never build. Move a case to positive when it is supported.
//!
//! Stock rustc is never invoked by this test.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

const STOCK: &str = include_str!("fixtures/trait_objects/stock.tsv");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("jr-trait-objects-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("exclusive scratch");
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/trait_objects")
}

struct Case {
    path: String,
    accept: bool,
}

fn cases() -> Vec<Case> {
    STOCK
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert_eq!(cols.len(), 3, "bad stock row {l:?}");
            Case {
                path: cols[0].into(),
                accept: match cols[1] {
                    "accept" => true,
                    "reject" => false,
                    other => panic!("bad stock verdict {other}"),
                },
            }
        })
        .collect()
}

fn native(source: &Path, output: Option<&Path>, workers: usize) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command
        .arg(source)
        .args(["--workers", &workers.to_string()]);
    match output {
        Some(path) => command.arg("-o").arg(path),
        None => command.arg("--check"),
    };
    command.output().expect("spawn jr-rust")
}

#[test]
fn stock_receipt_covers_every_fixture() {
    let listed: Vec<String> = cases().into_iter().map(|c| c.path).collect();
    for class in ["positive", "negative", "boundary"] {
        for entry in fs::read_dir(fixtures().join(class)).unwrap() {
            let name = entry.unwrap().file_name().into_string().unwrap();
            if let Some(stem) = name.strip_suffix(".rs") {
                let rel = format!("{class}/{stem}.rs");
                assert!(listed.contains(&rel), "{rel} missing from stock.tsv");
            }
        }
    }
    for case in cases() {
        let expect_accept = !case.path.starts_with("negative/");
        assert_eq!(
            case.accept, expect_accept,
            "{}: class disagrees with stock verdict",
            case.path
        );
        if case.accept {
            assert!(
                fixtures()
                    .join(case.path.replace(".rs", ".stdout"))
                    .exists(),
                "{}: missing stock stdout",
                case.path
            );
        }
    }
}

#[test]
fn native_matches_stock_workers_1_and_8() {
    let mut failures = Vec::new();
    for case in cases() {
        let source = fixtures().join(&case.path);
        for workers in [1, 8] {
            let scratch = Scratch::new();
            let exe = scratch.0.join("program");
            let check = native(&source, None, workers);
            let build = native(&source, Some(&exe), workers);
            let context = format!("{} workers={workers}", case.path);
            let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
            if case.path.starts_with("positive/") {
                if !check.status.success() || !build.status.success() {
                    failures.push(format!(
                        "{context}: native rejected a supported program: {}{stderr}",
                        String::from_utf8_lossy(&check.stderr)
                    ));
                    continue;
                }
                let run = Command::new(&exe).output().expect("run executable");
                let expected =
                    fs::read(fixtures().join(case.path.replace(".rs", ".stdout"))).unwrap();
                if !run.status.success() || run.stdout != expected {
                    failures.push(format!(
                        "{context}: output differs from stock: {:?}",
                        String::from_utf8_lossy(&run.stdout)
                    ));
                }
            } else {
                if check.status.success() || build.status.success() || exe.exists() {
                    failures.push(format!("{context}: native accepted"));
                    continue;
                }
                if check.status.code() != Some(1) || build.status.code() != Some(1) {
                    failures.push(format!("{context}: rejection was not a diagnostic exit"));
                }
                if case.path.starts_with("boundary/") && !stderr.contains("unsupported") {
                    failures.push(format!(
                        "{context}: boundary must fail explicitly unsupported: {stderr}"
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
