//! Nested runtime array reads and writes (NESTED-INDEX.md). Frozen stock sources under
//! tests/fixtures/nested_index with rustc 1.98.1 oracle stdout. Each runtime
//! index level is evaluated and bounds-checked outermost first.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!(
                "jr-nested-index-{}-{epoch}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch: {error}"),
            }
        }
        panic!("could not allocate scratch");
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn source(&self, source: &str) -> PathBuf {
        let path = self.path("input.rs");
        new_file(&path).write_all(source.as_bytes()).unwrap();
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
        .create_new(true)
        .write(true)
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
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&stdout))
        .stderr(new_file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {command:?}: {error}")),
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
fn compiler(input: &Path, output: &Path, workers: usize, check: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command.arg(input).args(["--workers", &workers.to_string()]);
    if check {
        command.arg("--check");
    } else {
        command.arg("-o").arg(output);
    }
    command
}
fn fixture(name: &str) -> (String, String) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nested_index");
    (
        fs::read_to_string(dir.join(format!("{name}.rs"))).unwrap(),
        fs::read_to_string(dir.join(format!("{name}.stdout"))).unwrap(),
    )
}
fn runs_with_exit(source: &str, expected: &str, expected_exit: i32) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("program");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                result.status.success(),
                "workers={workers}, check={check}: {}\n{source}",
                String::from_utf8_lossy(&result.stderr)
            );
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let result = capture(&mut Command::new(&output), &scratch, 5);
        assert_eq!(result.status.code(), Some(expected_exit), "{result:?}");
        assert_eq!(
            String::from_utf8_lossy(&result.stdout),
            expected,
            "workers={workers}"
        );
    }
}

#[test]
fn nested_reads_evaluate_and_check_each_level_in_order() {
    let (source, expected) = fixture("reads_order");
    runs_with_exit(&source, &expected, 0);
}

#[test]
fn nested_reads_through_fields_bools_and_whole_rows() {
    let (source, expected) = fixture("reads_fields");
    runs_with_exit(&source, &expected, 0);
}

#[test]
fn inner_level_trap_after_both_indexes_evaluated() {
    // Trap/stdout parity only: native trap 101 versus stock abort 134.
    let (source, expected) = fixture("trap_inner");
    runs_with_exit(&source, &expected, 101);
}

#[test]
fn outer_level_trap_before_inner_index_is_evaluated() {
    // Trap/stdout parity only: native trap 101 versus stock abort 134.
    let (source, expected) = fixture("trap_outer");
    runs_with_exit(&source, &expected, 101);
}

#[test]
fn nested_writes_value_then_each_index_in_order() {
    // Trap/stdout parity only for the final OOB write: native 101, stock 134.
    let (source, expected) = fixture("writes_order");
    runs_with_exit(&source, &expected, 101);
}

#[test]
fn nested_compound_write_outer_trap_before_inner_index() {
    // Trap/stdout parity only: native 101 versus stock abort 134.
    let (source, expected) = fixture("writes_compound_trap");
    runs_with_exit(&source, &expected, 101);
}

#[test]
fn nested_writes_all_ops_fields_bools_and_three_levels() {
    let (source, expected) = fixture("writes_ops");
    runs_with_exit(&source, &expected, 0);
}

#[test]
fn fixtures_are_frozen() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/nested_index");
    let sums = fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 14);
    let oracle = fs::read_to_string(dir.join("ORACLE")).unwrap();
    assert!(oracle.starts_with("rustc 1.98.1 (48a229cea 2026-09-01)"));
}
