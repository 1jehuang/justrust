//! Temporary-rooted assignment places (TEMP-PLACES.md). Frozen stock
//! sources under tests/fixtures/temp_places with rustc 1.98.1 oracle
//! stdout. Stock order is value, then carrier call, then runtime index.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
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
                "jr-temp-places-{}-{epoch}-{id}",
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
    pid: u32,
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
    let pid = child.0.id();
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
        pid,
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
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/temp_places");
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
fn value_then_carrier_then_index_order_matches_stock() {
    let (source, expected) = fixture("order");
    runs_with_exit(&source, &expected, 0);
}

#[test]
fn rhs_reads_precede_carrier_borrow() {
    let (source, expected) = fixture("reads_before_borrow");
    runs_with_exit(&source, &expected, 0);
}

#[test]
fn runtime_index_compound_traps_after_value_carrier_and_index() {
    let (source, expected) = fixture("index_trap");
    let scratch = Scratch::new();
    let input = scratch.source(&source);
    let stock = scratch.path("stock");
    let built = capture(
        Command::new(stock_rustc::stock_rustc())
            .args(["--edition=2024", "-Cpanic=abort"])
            .arg(&input)
            .arg("-o")
            .arg(&stock),
        &scratch,
        20,
    );
    assert!(built.status.success(), "{built:?}");
    let run = |path: &Path| {
        let mut command = Command::new("sh");
        command
            .args(["-c", "ulimit -c 0; exec \"$1\"", "temp-place-panic"])
            .arg(path)
            .env("RUST_BACKTRACE", "0");
        capture(&mut command, &scratch, 5)
    };
    let stock = run(&stock);
    assert_eq!(stock.status.signal(), Some(6), "{stock:?}");
    assert_eq!(stock.stdout, expected.as_bytes());
    let diagnostic = |result: &Captured| {
        let text = String::from_utf8(result.stderr.clone()).unwrap();
        let identity = format!("thread 'main' ({})", result.pid);
        assert!(text.contains(&identity), "{text}");
        text.replace(&identity, "thread 'main' (<PID>)")
    };
    let expected_diagnostic = diagnostic(&stock);
    assert!(expected_diagnostic.contains("index out of bounds"));
    for workers in [1, 8] {
        let output = scratch.path(&format!("native-{workers}"));
        for check in [true, false] {
            let built = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                built.status.success(),
                "workers={workers}, check={check}: {built:?}"
            );
            if check {
                assert!(!output.exists(), "CHECK published an executable");
            }
        }
        let actual = run(&output);
        assert_eq!(
            actual.status.signal(),
            Some(6),
            "workers={workers}: {actual:?}"
        );
        assert_eq!(actual.stdout, stock.stdout, "workers={workers}");
        assert_eq!(
            diagnostic(&actual),
            expected_diagnostic,
            "workers={workers}"
        );
    }
}

#[test]
fn fixtures_are_frozen() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/temp_places");
    let sums = fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
    assert_eq!(sums.lines().count(), 6);
    let oracle = fs::read_to_string(dir.join("ORACLE")).unwrap();
    assert!(oracle.starts_with("rustc 1.98.1 (48a229cea 2026-09-01)"));
}
