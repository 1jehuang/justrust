//! Shared stock-oracle harness for small whole-program cases.
//!
//! Each case is compiled natively with `jr-rust` and its stdout/exit status or
//! rejection is compared against the recorded expectation. When stock rustc is
//! installed the same source is also compiled with it (edition 2024,
//! `-Cpanic=abort`) and must agree, so expectations stay measured rather than
//! assumed. Stock rustc is an external oracle only, never a compiler input.
#![allow(dead_code)]
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expect {
    /// Both compilers accept; the program prints exactly this and exits 0.
    Run(&'static str),
    /// Stock rejects with this diagnostic code/fragment; native rejects too,
    /// without an "unsupported" boundary diagnostic.
    Invalid(&'static str),
}

pub struct Case {
    pub name: &'static str,
    pub source: &'static str,
    pub expect: Expect,
}

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);
impl Scratch {
    fn new(tag: &str) -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = base.join(format!("jr-oracle-{tag}-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
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

fn run(image: &Path) -> Output {
    Command::new(image).output().expect("run native image")
}

fn stock_available() -> bool {
    Command::new("rustc")
        .arg("-vV")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Run every case against jr-rust (and stock rustc when installed).
pub fn check(tag: &str, cases: &[Case]) {
    let stock = stock_available();
    let mut failures = Vec::new();
    for case in cases {
        let scratch = Scratch::new(tag);
        let source = scratch.path("main.rs");
        fs::write(&source, case.source).unwrap();
        if stock {
            let image = scratch.path("stock");
            let output = Command::new("rustc")
                .env_remove("RUSTC_BOOTSTRAP")
                .args(["--edition=2024", "-Cpanic=abort", "--color=never"])
                .args(["--crate-name=oracle", "-Awarnings"])
                .arg(&source)
                .arg("-o")
                .arg(&image)
                .output()
                .unwrap();
            let stderr = String::from_utf8_lossy(&output.stderr);
            match case.expect {
                Expect::Run(stdout) => {
                    if !output.status.success() {
                        failures.push(format!("{}: stock rejected: {stderr}", case.name));
                        continue;
                    }
                    let ran = run(&image);
                    if ran.stdout != stdout.as_bytes() || !ran.status.success() {
                        failures.push(format!("{}: stock output {:?}", case.name, ran));
                        continue;
                    }
                }
                Expect::Invalid(fragment) => {
                    if output.status.success() || !stderr.contains(fragment) {
                        failures.push(format!(
                            "{}: stock expected {fragment}: {stderr}",
                            case.name
                        ));
                        continue;
                    }
                }
            }
        }
        let image = scratch.path("native");
        let output = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&source)
            .arg("-o")
            .arg(&image)
            .output()
            .unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        if stderr.contains("panicked at") {
            failures.push(format!("{}: native compiler panic: {stderr}", case.name));
            continue;
        }
        match case.expect {
            Expect::Run(stdout) => {
                if !output.status.success() {
                    failures.push(format!("{}: native rejected: {stderr}", case.name));
                    continue;
                }
                let ran = run(&image);
                if ran.stdout != stdout.as_bytes() || !ran.status.success() {
                    failures.push(format!("{}: native output {:?}", case.name, ran));
                }
            }
            Expect::Invalid(_) => {
                if output.status.success() {
                    failures.push(format!("{}: native accepted invalid program", case.name));
                } else if stderr.contains("unsupported") {
                    failures.push(format!(
                        "{}: invalid program hit an unsupported boundary: {stderr}",
                        case.name
                    ));
                } else if image.exists() {
                    failures.push(format!("{}: rejected but wrote an image", case.name));
                }
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} {tag} cases failed (stock oracle {}):\n{}",
        failures.len(),
        cases.len(),
        if stock { "on" } else { "unavailable" },
        failures.join("\n")
    );
}
