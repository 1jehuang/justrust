//! Stock-first `mut` by-value parameter acceptance. `fixtures/mut_params/stock.json`
//! was produced by stock rustc before any native expectation was written.
//! Native CHECK and BUILD run through the public `jr-rust` CLI at workers 1
//! and 8; built executables must reproduce stock stdout byte for byte.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const RECEIPT: &str = include_str!("fixtures/mut_params/stock.json");
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = root.join(format!("jr-mut-params-{}-{id}", std::process::id()));
        fs::create_dir(&path).expect("exclusive scratch");
        Self(path)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Case {
    path: String,
    category: String,
    sha256: String,
    compile_status: i64,
    stdout: Vec<u8>,
}

/// Minimal extraction from the fixed receipt format; asserts its own shape.
fn cases() -> Vec<Case> {
    let field = |chunk: &str, key: &str| -> Option<String> {
        let marker = format!("\"{key}\": ");
        let start = chunk.find(&marker)? + marker.len();
        let rest = &chunk[start..];
        let end = rest.find([',', '\n']).unwrap_or(rest.len());
        Some(rest[..end].trim().trim_matches('"').to_string())
    };
    let body = &RECEIPT[RECEIPT.find("\"cases\": [").expect("cases")..];
    let mut cases = Vec::new();
    for chunk in body.split("\n    {").skip(1) {
        let stdout_hex = field(chunk, "stdout_hex").unwrap_or_default();
        let stdout = (0..stdout_hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&stdout_hex[i..i + 2], 16).unwrap())
            .collect();
        cases.push(Case {
            path: field(chunk, "path").expect("path"),
            category: field(chunk, "category").expect("category"),
            sha256: field(chunk, "sha256").expect("sha256"),
            compile_status: field(chunk, "compile_status").unwrap().parse().unwrap(),
            stdout,
        });
    }
    cases
}

fn sha256(path: &Path) -> String {
    let out = Command::new("sha256sum").arg(path).output().unwrap();
    String::from_utf8(out.stdout).unwrap()[..64].to_string()
}

fn jr_rust() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_jr-rust"))
}

#[test]
fn mut_parameters_match_frozen_stock() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mut_params");
    let cases = cases();
    assert_eq!(cases.len(), 9);
    assert_eq!(cases.iter().filter(|c| c.category == "positive").count(), 5);
    assert_eq!(cases.iter().filter(|c| c.category == "negative").count(), 3);
    assert_eq!(cases.iter().filter(|c| c.category == "boundary").count(), 1);
    for case in &cases {
        let source = root.join(&case.path);
        assert_eq!(sha256(&source), case.sha256, "{} changed", case.path);
        let positive = case.category == "positive";
        // Stock agreement: positives and the legal boundary compile, negatives do not.
        assert_eq!(
            case.compile_status == 0,
            case.category != "negative",
            "{}",
            case.path
        );
        for workers in ["1", "8"] {
            let scratch = Scratch::new();
            let output = scratch.0.join("program");
            for check in [true, false] {
                let mut command = Command::new(jr_rust());
                command.arg(&source).args(["--workers", workers]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&output);
                }
                let result = command.output().unwrap();
                let stderr = String::from_utf8_lossy(&result.stderr).to_lowercase();
                assert!(!stderr.contains("panicked"), "{}: {stderr}", case.path);
                match case.category.as_str() {
                    "positive" => assert!(result.status.success(), "{}: {stderr}", case.path),
                    "negative" => {
                        assert_eq!(result.status.code(), Some(1), "{}", case.path);
                        let expected = if case.path.ends_with("mut_mut.rs") {
                            "`mut` must be followed by a binding name"
                        } else {
                            "immutablereassignment"
                        };
                        assert!(stderr.contains(expected), "{}: {stderr}", case.path);
                    }
                    // Legal Rust outside this slice: explicit early rejection,
                    // never counted as conformance.
                    _ => {
                        assert_eq!(result.status.code(), Some(1), "{}", case.path);
                        assert!(
                            stderr.contains("expected ascii identifier"),
                            "{}: {stderr}",
                            case.path
                        );
                    }
                }
                if check {
                    assert!(
                        !output.exists(),
                        "{}: check published executable",
                        case.path
                    );
                }
            }
            if positive {
                let run = Command::new(&output).output().unwrap();
                assert!(run.status.success(), "{}", case.path);
                assert_eq!(run.stdout, case.stdout, "{} workers={workers}", case.path);
            } else {
                assert!(
                    !output.exists(),
                    "{}: rejected source published executable",
                    case.path
                );
            }
        }
    }
}
