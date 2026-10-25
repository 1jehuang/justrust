//! Full-program target corpus (FULL-PROGRAMS.md). Each realistic single-file
//! program under tests/fixtures/full_programs is native CHECKed, BUILT and RUN,
//! and must match the stock rustc oracle stdout recorded beside it. The
//! manifest is a ratchet: `pass` programs must keep passing, `fail` programs
//! must be rejected with an explicit diagnostic (never miscompiled), and an
//! unexpected pass fails until the manifest is flipped to `pass`.
//! Stock rustc is never invoked by this test.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const MANIFEST: &str = include_str!("fixtures/full_programs/manifest.tsv");
static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/full_programs")
}

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
                "jr-full-programs-{}-{epoch}-{id}",
                std::process::id()
            ));
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

struct Captured {
    success: bool,
    code: Option<i32>,
    timed_out: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn capture(command: &mut Command, scratch: &Scratch, timeout: Duration) -> Captured {
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
    let (status, timed_out) = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break (Some(status), false);
        }
        if start.elapsed() > timeout {
            break (None, true);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    drop(child);
    Captured {
        success: status.is_some_and(|s| s.success()),
        code: status.and_then(|s| s.code()),
        timed_out,
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}

#[derive(Debug, PartialEq)]
enum Outcome {
    /// check, build and run succeeded with exactly the oracle stdout.
    Pass,
    /// Native compiler rejected the program with an explicit diagnostic.
    Rejected(String),
    /// Anything else: miscompile, crash, timeout, silent failure.
    Broken(String),
}

fn first_diagnostic(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let line = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    // Strip the "jr-rust: <path>: <path>:L:C: " prefix to keep the reason.
    let reason = line.rsplit(".rs:").next().unwrap_or(line);
    let reason = reason
        .split_once(": ")
        .map(|(loc, rest)| {
            if loc.chars().all(|c| c.is_ascii_digit() || c == ':') {
                rest
            } else {
                reason
            }
        })
        .unwrap_or(reason);
    reason.chars().take(200).collect()
}

fn run_program(name: &str, scratch: &Scratch) -> Outcome {
    let one = run_program_with_workers(name, scratch, 1);
    let eight = run_program_with_workers(name, scratch, 8);
    match (one, eight) {
        (Outcome::Pass, Outcome::Pass) => Outcome::Pass,
        (Outcome::Rejected(one), Outcome::Rejected(eight)) => {
            Outcome::Rejected(format!("workers 1: {one}; workers 8: {eight}"))
        }
        (one, eight) => Outcome::Broken(format!(
            "worker matrix did not pass or explicitly reject consistently: workers 1: {one:?}, workers 8: {eight:?}"
        )),
    }
}

fn run_program_with_workers(name: &str, scratch: &Scratch, workers: usize) -> Outcome {
    let source = fixtures().join(format!("{name}.rs"));
    let expected = fs::read(fixtures().join(format!("{name}.stdout"))).unwrap();
    let output = scratch.path(&format!("{name}-w{workers}"));
    let compiler = env!("CARGO_BIN_EXE_jr-rust");
    let check = capture(
        Command::new(compiler)
            .arg(&source)
            .args(["--workers", &workers.to_string(), "--check"]),
        scratch,
        Duration::from_secs(60),
    );
    let build = capture(
        Command::new(compiler)
            .arg(&source)
            .args(["--workers", &workers.to_string(), "-o"])
            .arg(&output),
        scratch,
        Duration::from_secs(60),
    );
    for (mode, result) in [("check", &check), ("build", &build)] {
        if result.timed_out {
            return Outcome::Broken(format!("{mode} timed out"));
        }
    }
    match (check.success, build.success) {
        (false, false) => {
            let reason = first_diagnostic(&check.stderr);
            if reason.is_empty() || check.code != Some(1) || build.code != Some(1) {
                return Outcome::Broken(format!(
                    "rejection without explicit diagnostic (check {:?}, build {:?}): {reason}",
                    check.code, build.code
                ));
            }
            if output.exists() {
                return Outcome::Broken("failed build published an executable".into());
            }
            return Outcome::Rejected(reason);
        }
        (true, true) => {}
        (c, b) => {
            return Outcome::Broken(format!(
                "check and build disagree (check ok={c}, build ok={b}): {}{}",
                first_diagnostic(&check.stderr),
                first_diagnostic(&build.stderr)
            ));
        }
    }
    let run = capture(&mut Command::new(&output), scratch, Duration::from_secs(20));
    if run.timed_out {
        return Outcome::Broken("executable timed out".into());
    }
    if !run.success {
        return Outcome::Broken(format!(
            "executable exited {:?}: {}",
            run.code,
            String::from_utf8_lossy(&run.stderr)
        ));
    }
    if run.stdout != expected {
        return Outcome::Broken(format!(
            "MISCOMPILE: stdout differs from stock rustc\n--- expected\n{}\n--- native\n{}",
            String::from_utf8_lossy(&expected),
            String::from_utf8_lossy(&run.stdout)
        ));
    }
    if !run.stderr.is_empty() {
        return Outcome::Broken("executable wrote to stderr".into());
    }
    Outcome::Pass
}

struct Entry {
    name: String,
    expect_pass: bool,
    needs: Vec<String>,
}

fn manifest() -> Vec<Entry> {
    MANIFEST
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .map(|l| {
            let cols: Vec<&str> = l.split('\t').collect();
            assert!(cols.len() >= 2, "bad manifest line: {l:?}");
            let expect_pass = match cols[1] {
                "pass" => true,
                "fail" => false,
                other => panic!("bad expectation {other:?} in {l:?}"),
            };
            let needs = cols
                .get(2)
                .map(|n| {
                    n.split(',')
                        .filter(|t| !t.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            Entry {
                name: cols[0].to_owned(),
                expect_pass,
                needs,
            }
        })
        .collect()
}

#[test]
fn manifest_covers_every_fixture() {
    let mut listed: Vec<String> = manifest().into_iter().map(|e| e.name).collect();
    let mut on_disk: Vec<String> = fs::read_dir(fixtures())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter_map(|n| n.strip_suffix(".rs").map(str::to_owned))
        .collect();
    listed.sort();
    on_disk.sort();
    assert_eq!(listed, on_disk, "manifest.tsv must list every program once");
    for name in &on_disk {
        assert!(
            fixtures().join(format!("{name}.stdout")).exists(),
            "{name}: missing stock oracle stdout"
        );
    }
    for entry in manifest() {
        assert!(
            !entry.expect_pass || entry.needs.is_empty(),
            "{}: passing program still lists needed features {:?}",
            entry.name,
            entry.needs
        );
    }
}

#[test]
fn full_programs_ratchet() {
    let entries = manifest();
    let scratch = Scratch::new();
    let results: Vec<(&Entry, Outcome)> = std::thread::scope(|s| {
        let handles: Vec<_> = entries
            .iter()
            .map(|e| {
                let scratch = &scratch;
                s.spawn(move || (e, run_program(&e.name, scratch)))
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let mut report = String::new();
    let mut failures = Vec::new();
    let mut passing = 0;
    for (entry, outcome) in &results {
        let line = match outcome {
            Outcome::Pass => {
                passing += 1;
                if !entry.expect_pass {
                    failures.push(format!(
                        "{}: now PASSES; flip it to `pass` in manifest.tsv and clear its needs",
                        entry.name
                    ));
                }
                format!("PASS     {}", entry.name)
            }
            Outcome::Rejected(reason) => {
                if entry.expect_pass {
                    failures.push(format!("{}: REGRESSED, was passing: {reason}", entry.name));
                }
                format!("REJECTED {}: {reason}", entry.name)
            }
            Outcome::Broken(reason) => {
                failures.push(format!("{}: {reason}", entry.name));
                format!("BROKEN   {}: {reason}", entry.name)
            }
        };
        report.push_str(&line);
        report.push('\n');
    }
    eprintln!("full programs: {passing}/{} pass\n{report}", results.len());
    assert!(
        failures.is_empty(),
        "full-program ratchet:\n{}",
        failures.join("\n")
    );
}
