//! Stock-first normal Drop integration. No rejection of a Rust-valid unsupported
//! program counts as language conformance. Run this same target in bootstrap
//! debug and release profiles; the native compiler never delegates to rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use stock_rustc::stock_rustc;

use justrust_engine::compiler;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const INDEX: &str = include_str!("fixtures/drop_normal/receipts-v9/fixture-index.tsv");
const MANIFEST_HASH: &str = "afc7b27b95496aa84316d9e21485fb7c4f2c18d57701251bcffc152593aee7fe";
const INDEX_HASH: &str = "213e1c312094ffb078cd49c04e508721188e4b0308b1c990b939ce3af5b585e7";
const RECEIPT_HASH: &str = "a35fe212113376b84ed3b011f149b72a15001d2c78c76e3d38acd5e6427c45c1";
/// Byte hash of the exact stock rustc 1.98.1 (commit 48a229cea) binaries the
/// receipts were measured with: the Arch Linux package build and the official
/// rustup x86_64-unknown-linux-gnu build installed on hosted build machines.
/// Both report the same version, commit and LLVM 22.1.8.
const RUSTC_HASHES: &[&str] = &[
    "c916a85b7d4f52d9650399eebd15c672377933e6c2d8e98215757ab0b9b37136",
    "859254978c0a0402c32f949f6de0d99aee73be8d15f45aac00ae1448aac51e74",
];
const CHILD: &str = "JR_DROP_NORMAL_VERIFIED_STOCK_CHILD";
static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let root = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drop_normal")
            });
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "run-drop-normal-{}-{epoch}-{id}",
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
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl std::fmt::Display for Captured {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
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
    assert_eq!(
        result.status.code(),
        Some(1),
        "{context}: expected normal rejection: {result}"
    );
    assert!(
        !result.stderr.is_empty(),
        "{context}: missing diagnostic: {result}"
    );
}
fn check_hash(path: &Path, expected: &str, scratch: &Scratch) {
    let result = capture(Command::new("sha256sum").arg(path), scratch, 10);
    successful(&result, "frozen content hash");
    let output = String::from_utf8(result.stdout).unwrap();
    assert_eq!(
        output.split_whitespace().next(),
        Some(expected),
        "{}",
        path.display()
    );
}
fn unhex(value: &str) -> Vec<u8> {
    if value == "-" {
        return Vec::new();
    }
    assert_eq!(value.len() % 2, 0);
    (0..value.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&value[i..i + 2], 16).unwrap())
        .collect()
}
struct Case {
    category: String,
    path: String,
    hash: String,
    source: String,
    compile_status: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn corpus_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/drop_normal")
}
fn cases() -> Vec<Case> {
    assert_eq!(
        INDEX.lines().next(),
        Some(format!("# manifest_sha256={MANIFEST_HASH}").as_str())
    );
    INDEX
        .lines()
        .skip(1)
        .map(|line| {
            let fields: Vec<_> = line.split('\t').collect();
            assert_eq!(fields.len(), 7);
            let invalid = fields[0] == "negative";
            assert!(matches!(fields[0], "positive" | "negative" | "unsupported"));
            assert_eq!(fields[3], if invalid { "1" } else { "0" });
            assert_eq!(fields[4], if invalid { "none" } else { "0" });
            Case {
                category: fields[0].into(),
                path: fields[1].into(),
                hash: fields[2].into(),
                source: fs::read_to_string(corpus_root().join(fields[1])).unwrap(),
                compile_status: fields[3].parse().unwrap(),
                stdout: unhex(fields[5]),
                stderr: unhex(fields[6]),
            }
        })
        .collect()
}
fn fixture(case: &Case) -> Scratch {
    let scratch = Scratch::new();
    let source = scratch.write("main.rs", case.source.as_bytes());
    check_hash(&source, &case.hash, &scratch);
    scratch
}
fn execute(path: &Path, case: &Case, scratch: &Scratch, context: &str) {
    let bytes = fs::read(path).unwrap();
    assert!(bytes.len() >= 64, "{context}: truncated ELF");
    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01", "{context}");
    let result = capture(&mut Command::new(path), scratch, 10);
    successful(&result, context);
    assert_eq!(
        result.stdout, case.stdout,
        "{context}: exact stdout mismatch\n{result}"
    );
    assert_eq!(
        result.stderr, case.stderr,
        "{context}: exact stderr mismatch\n{result}"
    );
}
fn stock(case: &Case) {
    let scratch = fixture(case);
    for check in [true, false] {
        let output = scratch.path(if check { "stock.rmeta" } else { "stock" });
        let mut command = Command::new(stock_rustc());
        command
            .arg(scratch.path("main.rs"))
            .args([
                "--crate-name",
                "drop_fixture",
                "--edition=2024",
                "-C",
                "panic=abort",
                "-A",
                "warnings",
            ])
            .arg("-o")
            .arg(output);
        if check {
            command.arg("--emit=metadata");
        }
        let result = capture(&mut command, &scratch, 60);
        assert_eq!(
            result.status.code(),
            Some(case.compile_status),
            "{}: {result}",
            case.path
        );
        if case.category == "negative" {
            let expected = match case.path.as_str() {
                "negative/copy_drop.rs" | "negative/copy_drop_manual.rs" => "E0184",
                "negative/move_field.rs" => "E0509",
                _ => "E0040",
            };
            assert!(
                String::from_utf8_lossy(&result.stderr).contains(expected),
                "{}: {result}",
                case.path
            );
        }
        if check || case.category == "negative" {
            assert!(!scratch.path("stock").exists());
        }
    }
    if case.category != "negative" {
        execute(&scratch.path("stock"), case, &scratch, "stock runtime");
    }
}
fn attribute_blocked(case: &Case) -> bool {
    matches!(
        case.path.as_str(),
        "positive/scope.rs"
            | "positive/conditional.rs"
            | "positive/moves_returns.rs"
            | "positive/loops.rs"
            | "positive/labeled_continue.rs"
            | "positive/temporaries.rs"
    )
}
fn native_accepted(case: &Case) -> bool {
    (case.category == "positive"
        || matches!(
            case.path.as_str(),
            "unsupported/borrow.rs"
                | "unsupported/generic_function.rs"
                | "unsupported/dead_generic.rs"
                | "unsupported/array.rs"
                | "unsupported/borrowed_replacement.rs"
                | "unsupported/dead_aggregate.rs"
                | "unsupported/enum.rs"
                | "unsupported/generic.rs"
                | "unsupported/nested.rs"
                | "unsupported/tuple.rs"
        ))
        && !attribute_blocked(case)
        && !path_blocked(case)
}
/// Unqualified `module::Trait::method` / `Trait::method` paths are existing
/// resolver boundaries unrelated to Drop. Frozen qualified variants carry the
/// native claims instead; a resolver rejection never counts as E0040
/// conformance. `parameter_paths.rs` (a `mut` by-value parameter) is native
/// positive since MUT-PARAMS support.
fn path_blocked(case: &Case) -> bool {
    matches!(case.path.as_str(), "negative/explicit_ufcs.rs")
}
/// Rejected by earlier parser/library boundaries before Drop admission runs:
/// `String::from` (no alloc/std String) and nominal lifetime parameters.
/// These neither claim nor refute Drop behavior.
fn precedent_blocked(case: &Case) -> bool {
    matches!(case.path.as_str(), "unsupported/reference_field.rs")
}
fn rejection_reason(message: &str, case: &Case) {
    let lower = message.to_ascii_lowercase();
    for forbidden in [
        "resource",
        "budget",
        "allocation failed",
        "out of memory",
        "contract:",
        "output already exists",
        "no such file",
        "permission denied",
        "internal",
        "panicked",
    ] {
        assert!(
            !lower.contains(forbidden),
            "{}: infrastructure failure is not source rejection: {message}",
            case.path
        );
    }
    // Originals retain their exact frozen attributes. The separately frozen
    // *_plain variants are native-positive, not silent source rewrites.
    if attribute_blocked(case) {
        assert!(
            lower.contains("unsupported")
                && (lower.contains("attribute") || lower.contains("derive")),
            "{}: expected explicit unsupported attribute boundary: {message}",
            case.path
        );
        return;
    }
    if path_blocked(case) {
        assert!(
            lower.contains("non-module declaration in path")
                || lower.contains("unresolved function"),
            "{}: expected explicit parser/resolver boundary: {message}",
            case.path
        );
        return;
    }
    if precedent_blocked(case) {
        let expected = match case.path.as_str() {
            "negative/move_field.rs" => "unresolved type",
            _ => "lifetime parameters on nominal and impl declarations are unsupported",
        };
        assert!(
            lower.contains(expected),
            "{}: expected precedent syntax boundary `{expected}`: {message}",
            case.path
        );
        return;
    }
    // Stock E0509 requires a non-Copy field, outside the scalar owner domain.
    // This case checks an explicit unsupported owner/type boundary, not native
    // field-move language conformance or diagnostic precedence.
    if case.category == "unsupported" {
        assert!(
            lower.contains("unsupported"),
            "{}: excluded domain requires explicit unsupported diagnostic: {message}",
            case.path
        );
        let relevant = lower.contains("drop") || lower.contains("destructor");
        assert!(
            relevant,
            "{}: unrelated unsupported boundary: {message}",
            case.path
        );
        return;
    }
    let expected = match case.path.as_str() {
        "negative/copy_drop_manual.rs" | "negative/copy_drop.rs" => {
            lower.contains("copy") && lower.contains("drop") && lower.contains("same type")
        }
        "negative/move_field.rs" => lower.contains("e0509"),
        "negative/explicit_method.rs"
        | "negative/explicit_qualified.rs"
        | "negative/generic_explicit_ufcs.rs" => {
            (lower.contains("explicit") || lower.contains("direct"))
                && (lower.contains("drop") || lower.contains("destructor"))
        }
        _ => false,
    };
    assert!(
        expected,
        "{}: expected specific semantic rejection, got {message}",
        case.path
    );
    assert!(
        !lower.contains("unsupported"),
        "{}: unsupported is not invalid-Rust conformance: {message}",
        case.path
    );
}
fn native(case: &Case) {
    let accepted = native_accepted(case);
    eprintln!("native case {} accepted={accepted}", case.path);
    for workers in [1, 8] {
        for check in [true, false] {
            let scratch = fixture(case);
            let context = format!("{} workers={workers} check={check}", case.path);
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(scratch.path("main.rs"))
                .args(["--workers", &workers.to_string()]);
            if check {
                command.arg("--check");
            } else {
                command.arg("-o").arg(scratch.path("native"));
            }
            let cli = capture(&mut command, &scratch, 30);
            if accepted {
                successful(&cli, &context);
            } else {
                rejected(&cli, &context);
                rejection_reason(&String::from_utf8_lossy(&cli.stderr), case);
            }
            if check || !accepted {
                assert!(
                    !scratch.path("native").exists(),
                    "{context}: executable leaked"
                );
            } else {
                execute(&scratch.path("native"), case, &scratch, &context);
            }
            let observed = compiler::compile_observed(case.source.clone(), workers, check);
            if accepted {
                let compiled = observed.unwrap_or_else(|e| panic!("{context}: {e}"));
                assert!(compiled.report.capture_complete, "{context}");
                assert_eq!(compiled.report.events_dropped, 0, "{context}");
                assert_eq!(compiled.report.failures, 0, "{context}");
                assert!(compiled.analyses.iter().all(|a| a.passed()), "{context}");
                assert_eq!(compiled.analyses.len(), compiled.functions, "{context}");
                assert_eq!(compiled.control_flow.len(), compiled.functions, "{context}");
                assert!(
                    compiled.resource_usage.first_failure.is_none(),
                    "{context}: unexpected budget failure"
                );
                assert!(
                    !compiled.resource_usage.poisoned,
                    "{context}: poisoned successful compilation"
                );
                assert_eq!(compiled.executable.is_none(), check, "{context}");
                if check {
                    assert!(
                        !compiled
                            .report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 103),
                        "{context}: CHECK evaluated machine-code node"
                    );
                }
                if let Some(bytes) = compiled.executable {
                    let path = scratch.write("observed", &bytes);
                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
                    execute(&path, case, &scratch, &context);
                }
            } else {
                let failure = observed
                    .err()
                    .unwrap_or_else(|| panic!("{context}: unexpected observed acceptance"));
                rejection_reason(&failure.to_string(), case);
                assert!(
                    failure.resource_usage.first_failure.is_none(),
                    "{context}: budget exhaustion is not semantic rejection"
                );
                assert!(
                    !failure.resource_usage.poisoned,
                    "{context}: poisoned rejection ledger"
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
fn frozen_stock_before_native_normal_drop() {
    let cases = cases();
    assert_eq!(cases.len(), 56);
    assert_eq!(
        cases.iter().filter(|c| c.category == "positive").count(),
        33
    );
    assert_eq!(cases.iter().filter(|c| c.category == "negative").count(), 7);
    assert_eq!(
        cases.iter().filter(|c| c.category == "unsupported").count(),
        16
    );
    assert_eq!(
        cases.iter().filter(|case| native_accepted(case)).count(),
        37
    );
    assert_eq!(
        cases.iter().filter(|case| attribute_blocked(case)).count(),
        6
    );
    if std::env::var(CHILD).ok().as_deref() == Some(MANIFEST_HASH) {
        // Exercise genuine positives before compatibility-domain diagnostics.
        for case in cases.iter().filter(|case| native_accepted(case)) {
            native(case);
        }
        for case in cases.iter().filter(|case| !native_accepted(case)) {
            native(case);
        }
        return;
    }
    let scratch = Scratch::new();
    let root = corpus_root();
    check_hash(
        &root.join("receipts-v9/manifest.json"),
        MANIFEST_HASH,
        &scratch,
    );
    check_hash(
        &root.join("receipts-v9/fixture-index.tsv"),
        INDEX_HASH,
        &scratch,
    );
    check_hash(&root.join("receipts-v9/stock.json"), RECEIPT_HASH, &scratch);
    let rustc_hash = capture(Command::new("sha256sum").arg(stock_rustc()), &scratch, 10);
    successful(&rustc_hash, "stock compiler content hash");
    let rustc_hash = String::from_utf8(rustc_hash.stdout).unwrap();
    assert!(
        RUSTC_HASHES.contains(&rustc_hash.split_whitespace().next().unwrap_or("")),
        "unrecognized stock rustc build: {rustc_hash}"
    );
    let version = capture(Command::new(stock_rustc()).arg("-Vv"), &scratch, 10);
    successful(&version, "stock compiler identity");
    assert!(
        String::from_utf8_lossy(&version.stdout)
            .contains("48a229ceaefd4985c50990b14116b6d856af0985")
    );
    // Run the complete exact-source stock corpus before native assertions, not
    // merely one stock case followed by that case's native execution.
    for case in &cases {
        stock(case);
    }
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "frozen_stock_before_native_normal_drop",
            "--nocapture",
        ])
        .env(CHILD, MANIFEST_HASH);
    let result = capture(&mut child, &scratch, 360);
    successful(
        &result,
        "bounded native CLI and observed CHECK/BUILD corpus child",
    );
}
