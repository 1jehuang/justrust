//! Scalar literal and range patterns (SCALAR-PATTERNS.md): char literals,
//! inclusive/exclusive/half-open integer and char ranges, `iN::MIN/MAX`
//! endpoints, and exact interval usefulness and exhaustiveness.
//! Fixtures are stock-verified by scripts/scalar_patterns_oracle.sh (rustc 1.98.1,
//! edition 2024, -C panic=abort): accept/*.rs run and print <name>.stdout, and
//! reject/*.rs are rejected by stock. Stock rustc is never invoked here.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn fixtures(kind: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/scalar_patterns")
        .join(kind)
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("jr-scalar-patterns-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("create scratch: {e}"),
            }
        }
        panic!("could not allocate scratch");
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn capture(command: &mut Command, scratch: &Scratch) -> Output {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    let open = |p: &Path| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(p)
            .unwrap()
    };
    command
        .stdin(Stdio::null())
        .stdout(open(&out))
        .stderr(open(&err));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {command:?}: {e}")),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "timed out: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    Output {
        status,
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}

fn sources(kind: &str) -> Vec<(String, PathBuf)> {
    let mut found: Vec<_> = fs::read_dir(fixtures(kind))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rs"))
        .map(|p| (p.file_stem().unwrap().to_str().unwrap().to_owned(), p))
        .collect();
    found.sort();
    found
}

fn compile(source: &Path, output: Option<&Path>, workers: usize, scratch: &Scratch) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command
        .arg(source)
        .args(["--workers", &workers.to_string()]);
    match output {
        Some(output) => command.arg("-o").arg(output),
        None => command.arg("--check"),
    };
    capture(&mut command, scratch)
}

#[test]
fn accepted_programs_match_stock_output_at_workers_one_and_eight() {
    let scratch = Scratch::new();
    let accepted = sources("accept");
    assert!(accepted.len() >= 6, "accept corpus shrank");
    for (name, source) in &accepted {
        let expected = fs::read(fixtures("accept").join(format!("{name}.stdout")))
            .unwrap_or_else(|e| panic!("{name}: missing stock oracle stdout: {e}"));
        assert!(!expected.is_empty(), "{name}: empty oracle output");
        for workers in [1, 8] {
            let check = compile(source, None, workers, &scratch);
            assert!(
                check.status.success(),
                "{name} workers {workers} check: {}",
                String::from_utf8_lossy(&check.stderr)
            );
            let output = scratch.path(&format!("{name}-{workers}"));
            let build = compile(source, Some(&output), workers, &scratch);
            assert!(
                build.status.success(),
                "{name} workers {workers} build: {}",
                String::from_utf8_lossy(&build.stderr)
            );
            let run = capture(&mut Command::new(&output), &scratch);
            assert!(run.status.success(), "{name} workers {workers}: {run:?}");
            assert_eq!(
                String::from_utf8_lossy(&run.stdout),
                String::from_utf8_lossy(&expected),
                "{name} workers {workers}: native output differs from stock"
            );
            assert!(run.stderr.is_empty(), "{name}: wrote to stderr");
        }
    }
}

#[test]
fn stock_rejected_programs_are_rejected_with_a_diagnostic() {
    let scratch = Scratch::new();
    let rejected = sources("reject");
    assert!(rejected.len() >= 15, "reject corpus shrank");
    for (name, source) in &rejected {
        for workers in [1, 8] {
            let check = compile(source, None, workers, &scratch);
            let output = scratch.path(&format!("{name}-{workers}"));
            let build = compile(source, Some(&output), workers, &scratch);
            for (mode, result) in [("check", &check), ("build", &build)] {
                assert_eq!(
                    result.status.code(),
                    Some(1),
                    "{name} workers {workers} {mode} must be rejected: {result:?}"
                );
                let stderr = String::from_utf8_lossy(&result.stderr);
                assert!(
                    stderr.starts_with("jr-rust: "),
                    "{name} {mode}: missing diagnostic: {stderr}"
                );
                assert!(
                    !stderr.contains("character literal and range patterns are unsupported"),
                    "{name} {mode}: rejected as unsupported, not as invalid: {stderr}"
                );
            }
            assert!(
                !output.exists(),
                "{name}: failed build published an executable"
            );
        }
    }
}
