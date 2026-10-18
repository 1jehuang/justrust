//! Frozen stock-first oracle. Reproduce retained sources/receipts with
//! `python3 experiments/leading_zeros_oracle.py --output-dir NEW --freeze-dir NEW`.
//! Legal unsupported syntax is classified by stock but is not called invalid.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MANIFEST: &str = include_str!("fixtures/leading_zeros/manifest.tsv");
const PROVIDER: &[u8] = include_bytes!("../libraries/probes/estimator/estimator.rs");
const PROVIDER_HASH: &str = "2d109735cfff1317afd02e093012ee4d4c3b5bf39431da45f6afe99002b0f9b5";
const CHILD: &str = "JR_LEADING_ZEROS_NATIVE_CHILD";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "jr-leading-zeros-{}-{epoch}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create scratch: {e}"),
            }
        }
        panic!("could not create exclusive scratch")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        new_file(&path).write_all(bytes).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap()
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl std::fmt::Display for Captured {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "status: {}\nstdout:\n{}\nstderr:\n{}",
            self.status,
            String::from_utf8_lossy(&self.stdout),
            String::from_utf8_lossy(&self.stderr)
        )
    }
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&stdout))
        .stderr(new_file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|e| panic!("{command:?}: {e}")),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timed out: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    Captured {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn successful(result: &Captured, context: &str) {
    assert!(result.status.success(), "{context}: {result}");
}
fn rejected(result: &Captured, context: &str) {
    assert!(
        !result.status.success(),
        "{context}: unexpectedly accepted\n{result}"
    );
    assert!(
        result.status.code().is_some(),
        "{context}: signal: {result}"
    );
    assert!(
        !result.stderr.is_empty(),
        "{context}: no diagnostic\n{result}"
    );
    assert!(
        !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
        "{context}: compiler panic: {result}"
    );
}
fn semantic_rejection(message: &str, case: &str) {
    // The public boundary wraps ordinary source type errors in InvalidMetadata.
    // Require the semantic reason below rather than rejecting that wrapper.
    let lower = message.to_ascii_lowercase();
    for forbidden in [
        "resource",
        "budget",
        "allocation failed",
        "out of memory",
        "contract:",
        "cycle",
        "output already exists",
        "no such file",
        "permission denied",
        "internal",
        "panicked",
    ] {
        assert!(
            !lower.contains(forbidden),
            "{case}: infrastructure/resource failure masquerading as rejection: {message}"
        );
    }
    let expected = if case == "result" {
        (lower.contains("expected")
            && lower.contains("got")
            && lower.contains("i32")
            && lower.contains("u32"))
            || lower.contains("type mismatch")
    } else {
        lower.contains("no applicable method leading_zeros")
            || lower.contains("unsupported unresolved receiver type")
    };
    assert!(
        expected,
        "{case}: expected receiver/method or result-type rejection, got {message}"
    );
}
struct Case {
    name: String,
    category: String,
    source: String,
    expected: Vec<u8>,
    hash: String,
}
fn cases() -> Vec<Case> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/leading_zeros");
    MANIFEST
        .lines()
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 4);
            assert_eq!(
                fields[2],
                if fields[1] == "invalid" {
                    "rejected"
                } else {
                    "accepted"
                }
            );
            Case {
                name: fields[0].into(),
                category: fields[1].into(),
                hash: fields[3].into(),
                source: fs::read_to_string(root.join(format!("{}.rs", fields[0]))).unwrap(),
                expected: fs::read(root.join(format!("{}.stdout", fields[0]))).unwrap(),
            }
        })
        .collect()
}
fn fixture(case: &Case) -> Scratch {
    let scratch = Scratch::new();
    scratch.write("main.rs", case.source.as_bytes());
    if case.name == "estimator" {
        scratch.write("estimator.rs", PROVIDER);
    }
    scratch
}
fn check_hash(path: &Path, expected: &str, scratch: &Scratch) {
    let result = capture(Command::new("sha256sum").arg(path), scratch, 10);
    successful(&result, "frozen source hash");
    assert_eq!(
        String::from_utf8(result.stdout)
            .unwrap()
            .split_whitespace()
            .next(),
        Some(expected)
    );
}
fn execute(path: &Path, expected: &[u8], scratch: &Scratch, context: &str) {
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() >= 64, "{context}: truncated ELF");
    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01", "{context}");
    let result = capture(&mut Command::new(path), scratch, 10);
    successful(&result, context);
    assert_eq!(
        result.stdout, expected,
        "{context}: complete stdout\n{result}"
    );
    assert!(result.stderr.is_empty(), "{context}: {result}");
}
fn stock(case: &Case) {
    let scratch = fixture(case);
    check_hash(&scratch.path("main.rs"), &case.hash, &scratch);
    if case.name == "estimator" {
        check_hash(&scratch.path("estimator.rs"), PROVIDER_HASH, &scratch);
    }
    let accepted = case.category != "invalid";
    for check in [true, false] {
        let mut command = Command::new("/usr/bin/rustc");
        command
            .current_dir(&scratch.0)
            .args(["--edition=2024", "main.rs", "-o"]);
        command.arg(if check { "stock.rmeta" } else { "stock" });
        if check {
            command.arg("--emit=metadata");
        }
        let result = capture(&mut command, &scratch, 60);
        if accepted {
            successful(&result, &case.name);
        } else {
            rejected(&result, &case.name);
        }
        if check {
            assert!(!scratch.path("stock").exists());
        }
    }
    if accepted {
        execute(&scratch.path("stock"), &case.expected, &scratch, &case.name);
    } else {
        assert!(!scratch.path("stock").exists());
    }
}
fn native(case: &Case) {
    // These are stock-legal boundaries, not negative language tests.
    if case.category == "legal-unsupported" {
        return;
    }
    let accepted = case.category == "valid";
    for workers in [1, 8] {
        let scratch = fixture(case);
        for check in [true, false] {
            let context = format!("{} workers={workers} check={check}", case.name);
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(scratch.path("main.rs"))
                .args(["--workers", &workers.to_string()]);
            if check {
                command.arg("--check");
            } else {
                command.arg("-o").arg(scratch.path("native"));
            }
            let result = capture(&mut command, &scratch, 30);
            if accepted {
                successful(&result, &context);
            } else {
                rejected(&result, &context);
                semantic_rejection(&String::from_utf8_lossy(&result.stderr), &case.name);
            }
            if check || !accepted {
                assert!(
                    !scratch.path("native").exists(),
                    "{context}: published executable"
                );
            } else {
                execute(&scratch.path("native"), &case.expected, &scratch, &context);
            }

            let observed = if case.name == "estimator" {
                compiler::compile_file_observed(scratch.path("main.rs"), workers, check)
            } else {
                compiler::compile_observed(case.source.clone(), workers, check)
            };
            if accepted {
                let compiled = observed.unwrap_or_else(|e| panic!("{context}: {e}"));
                assert!(compiled.report.capture_complete, "{context}");
                assert_eq!(compiled.report.events_dropped, 0, "{context}");
                assert_eq!(compiled.report.failures, 0, "{context}");
                assert!(compiled.analyses.iter().all(|a| a.passed()), "{context}");
                assert_eq!(compiled.analyses.len(), compiled.functions, "{context}");
                assert_eq!(compiled.control_flow.len(), compiled.functions, "{context}");
                assert_eq!(compiled.executable.is_none(), check, "{context}");
                if case.name == "estimator" {
                    assert_eq!(compiled.source_files, 2);
                    assert_eq!(compiled.source_bytes, case.source.len() + PROVIDER.len());
                }
                if check {
                    assert!(
                        !compiled
                            .report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 103),
                        "{context}: check lowered machine code"
                    );
                }
                if let Some(bytes) = compiled.executable {
                    let path = scratch.write("observed", &bytes);
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                    execute(&path, &case.expected, &scratch, &context);
                }
            } else {
                let failure = observed
                    .err()
                    .unwrap_or_else(|| panic!("{context}: observed accepted invalid source"));
                semantic_rejection(&failure.to_string(), &case.name);
                assert!(
                    failure.resource_usage.first_failure.is_none(),
                    "{context}: budget failure is not semantic rejection"
                );
                assert!(
                    !failure.resource_usage.poisoned,
                    "{context}: poisoned resource ledger"
                );
                if let Some(report) = failure.report {
                    assert!(report.capture_complete, "{context}");
                    assert_eq!(report.events_dropped, 0, "{context}");
                }
            }
        }
    }
}

#[test]
fn frozen_stock_before_native() {
    let cases = cases();
    assert_eq!(cases.len(), 30);
    assert_eq!(cases.iter().filter(|c| c.category == "invalid").count(), 8);
    assert_eq!(
        cases
            .iter()
            .filter(|c| c.category == "legal-unsupported")
            .count(),
        2
    );
    if std::env::var_os(CHILD).is_some() {
        for case in &cases {
            native(case);
        }
        return;
    }
    let scratch = Scratch::new();
    let version = capture(Command::new("/usr/bin/rustc").arg("-Vv"), &scratch, 10);
    successful(&version, "stock rustc version");
    assert!(
        String::from_utf8(version.stdout)
            .unwrap()
            .contains("host: x86_64-unknown-linux-gnu")
    );
    // All frozen sources are independently reclassified before the first native
    // assertion. Retained stock receipts are evidence, not a replacement for this.
    for case in &cases {
        stock(case);
    }
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", "frozen_stock_before_native", "--nocapture"])
        .env(CHILD, "1");
    let result = capture(&mut child, &scratch, 240);
    successful(&result, "bounded native CLI and observed corpus child");
}
