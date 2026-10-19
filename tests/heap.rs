//! Heap lane: raw pointers, allocator and owning heap types.
//! Fixtures under tests/fixtures/heap carry stock rustc 1.98 (edition 2024,
//! -C panic=abort) stdout; every reject_* fixture is rejected by stock.
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
            let path = base.join(format!("jr-heap-{}-{epoch}-{id}", std::process::id()));
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
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/heap");

fn fixture(name: &str) -> String {
    fs::read_to_string(Path::new(FIXTURES).join(name)).unwrap()
}

fn runs(name: &str) {
    let source = fixture(&format!("{name}.rs"));
    let expected = fixture(&format!("{name}.stdout"));
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(&source);
        let output = scratch.path("program");
        let checked = capture(&mut compiler(&input, &output, workers, true), &scratch, 60);
        assert!(checked.status.success(), "{name} check: {checked:?}");
        let built = capture(&mut compiler(&input, &output, workers, false), &scratch, 60);
        assert!(built.status.success(), "{name} build: {built:?}");
        let run = capture(&mut Command::new(&output), &scratch, 20);
        assert_eq!(run.status.code(), Some(0), "{name} run: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{name}");
    }
}

fn rejects(name: &str, needle: &str) {
    let source = fixture(&format!("{name}.rs"));
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(&source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 60);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(!result.status.success(), "{name}: accepted");
            assert!(stderr.contains(needle), "{name}: {stderr}");
            assert!(!stderr.contains("panicked at"), "{name}: {stderr}");
            assert!(!output.exists(), "{name}: published executable");
        }
    }
}

#[test]
fn raw_pointer_reads_writes_casts_and_equality() {
    runs("raw_read_write");
}

#[test]
fn raw_pointer_deref_requires_unsafe() {
    rejects("reject_deref_outside_unsafe", "E0133");
}

#[test]
fn raw_pointer_deref_requires_unsafe_in_unused_function() {
    rejects("reject_unused_deref", "E0133");
}

#[test]
fn raw_pointer_const_is_not_writable() {
    rejects("reject_write_const", "*const");
}

#[test]
fn shared_reference_does_not_cast_to_mut_pointer() {
    rejects("reject_shared_to_mut", "E0606");
}
