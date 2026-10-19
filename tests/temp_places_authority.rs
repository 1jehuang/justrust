//! Temporary-rooted assignment places: invalid and legal-unsupported
//! sources must fail at CHECK and BUILD (workers 1 and 8) with no
//! executable, including when the offending function is unused.
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

// Stock: E0594.
#[test]
fn shared_reference_carrier_is_invalid() {
    let p = "fn p(x: &i32) -> &i32 { x }";
    let s = "struct S { f: i32 } fn q(s: &S) -> &S { s }";
    for (label, body) in [
        (
            "deref_assign",
            format!("{p} fn main() {{ let x = 1; *p(&x) = 2; }}"),
        ),
        (
            "deref_compound",
            format!("{p} fn main() {{ let x = 1; *p(&x) += 2; }}"),
        ),
        (
            "field_assign",
            format!("{s} fn main() {{ let s = S {{ f: 1 }}; q(&s).f = 2; }}"),
        ),
        (
            "field_compound",
            format!("{s} fn main() {{ let s = S {{ f: 1 }}; q(&s).f += 2; }}"),
        ),
        (
            "unused",
            format!("{p} fn bad() {{ let x = 1; *p(&x) += 1; }} fn main() {{}}"),
        ),
    ] {
        rejects(label, &body, "E0594");
    }
}

// Stock: E0596, E0308, E0368, E0614, E0609, E0616, E0277, E0502.
#[test]
fn ordinary_invalid_programs_are_rejected() {
    let p = "fn p(x: &mut i32) -> &mut i32 { x }";
    let s = "struct S { f: i32 } fn q(s: &mut S) -> &mut S { s }";
    for (label, body, needle) in [
        ("immutable_root", format!("{p} fn main() {{ let x = 1; *p(&mut x) = 2; }}"), "ImmutableReassignment"),
        ("immutable_unused", format!("{s} fn bad() {{ let s = S {{ f: 1 }}; q(&mut s).f += 1; }} fn main() {{}}"), "ImmutableReassignment"),
        ("suffix_mismatch", format!("{p} fn main() {{ let mut x = 1; *p(&mut x) = 2u8; }}"), "suffix"),
        ("bool_add", "fn p(x: &mut bool) -> &mut bool { x } fn main() { let mut x = true; *p(&mut x) += true; }".into(), "compound assignment requires"),
        ("scalar_deref", "fn p(x: &mut i32) -> i32 { *x } fn main() { let mut x = 1; *p(&mut x) = 2; }".into(), "E0614"),
        ("unknown_field", format!("{s} fn main() {{ let mut s = S {{ f: 1 }}; q(&mut s).g = 2; }}"), "unknown nominal field"),
        ("private_field", "mod m { pub struct S { f: i32 } pub fn p(s: &mut S) -> &mut S { s } pub fn mk() -> S { S { f: 1 } } } fn main() { let mut s = m::mk(); m::p(&mut s).f = 2; }".into(), "private field"),
        ("float_into_int", format!("{p} fn main() {{ let mut x = 1; *p(&mut x) += 1.5; }}"), "Float"),
        ("live_shared_loan", format!("{s} fn main() {{ let mut s = S {{ f: 1 }}; let r = &s; q(&mut s).f = 2; let _k = r.f; }}"), "BorrowConflict"),
        ("constant_oob", "struct S { a: [i32; 2] } fn p(s: &mut S) -> &mut S { s } fn main() { let mut s = S { a: [0; 2] }; p(&mut s).a[5] = 2; }".into(), "out of bounds"),
    ] {
        rejects(label, &body, needle);
    }
}

// Stock accepts these; they stay explicit Unsupported in this slice.
#[test]
fn legal_unsupported_forms_stay_explicit() {
    for (label, body) in [
        (
            "by_value_field",
            "struct S { f: i32 } fn mk() -> S { S { f: 1 } } fn main() { mk().f = 2; }",
        ),
        (
            "by_value_compound",
            "struct S { f: i32 } fn mk() -> S { S { f: 1 } } fn main() { mk().f += 2; }",
        ),
        (
            "plain_runtime_index",
            "struct S { g: [i32; 3] } fn ps(s: &mut S) -> &mut S { s } fn main() { let mut s = S { g: [0; 3] }; let k = 1; ps(&mut s).g[k] = 4; }",
        ),
    ] {
        rejects(label, body, "unsupported");
    }
}
