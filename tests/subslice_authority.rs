//! Shared subslices: invalid and legal-unsupported sources must
//! fail at CHECK and BUILD (workers 1 and 8) with no executable, including
//! when the offending function is unused.
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
                "jr-subslice-authority-{}-{epoch}-{id}",
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
    #[allow(dead_code)]
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
fn rejects(label: &str, source: &str, needle: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                !result.status.success() && result.status.code().is_some(),
                "{label}: workers={workers} check={check} {result:?}"
            );
            assert!(!stderr.contains("panicked at"), "{label}: {stderr}");
            assert!(
                stderr.contains(needle),
                "{label}: expected {needle:?}: {stderr}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn invalid_subslices_are_rejected() {
    for (label, body, needle) in [
        (
            "non_usize_bound",
            "fn main() { let a = [1i32, 2]; let s: &[i32] = &a; let k = 1i32; let t = &s[k..]; }",
            "usize",
        ),
        (
            "unused_bad_bound",
            "fn bad() { let a = [1i32, 2]; let s: &[i32] = &a; let t = &s[true..]; } fn main() {}",
            "usize",
        ),
        (
            "inclusive_without_end",
            "fn main() { let a = [1i32, 2]; let s: &[i32] = &a; let t = &s[1..=]; }",
            "end",
        ),
    ] {
        rejects(label, body, needle);
    }
}

// Stock accepts these; they stay explicit Unsupported in this slice.
#[test]
fn legal_unsupported_subslices_stay_explicit() {
    for (label, body) in [
        (
            "mutable_subslice",
            "fn main() { let mut a = [1i32, 2, 3]; let t = &mut a[1..]; t[0] = 5; }",
        ),
        (
            "str_slice",
            "fn main() { let s = \"hello\"; let t = &s[1..3]; }",
        ),
        (
            "unborrowed_range_value",
            "fn main() { let a = [1i32, 2, 3]; let s: &[i32] = &a; let t = s[1..]; }",
        ),
    ] {
        rejects(label, body, "nsupported");
    }
}
