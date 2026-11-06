//! Shared subslices `&s[a..b]` (SUBSLICE.md). Frozen stock sources under
//! tests/fixtures/subslice with rustc 1.98.1 oracle stdout. Each runtime
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
            let path = base.join(format!("jr-subslice-{}-{epoch}-{id}", std::process::id()));
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
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/subslice");
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
fn subslice_order_bounds_and_start_after_end_trap_match_stock() {
    // Trap/stdout parity only for the final start > end trap: 101 vs stock 134.
    let (source, expected) = fixture("order");
    runs_with_exit(&source, &expected, 101);
}

#[test]
fn subslice_all_range_kinds_fields_bytes_bools_and_chaining() {
    let (source, expected) = fixture("ops");
    runs_with_exit(&source, &expected, 0);
}

#[test]
fn every_out_of_range_class_traps_before_later_output() {
    for range in ["3..9", "6..", "..9", "..=5", "4..2"] {
        let source = format!(
            "fn main() {{ let a = [1i32, 2, 3, 4, 5]; let s: &[i32] = &a; println!(\"before\"); let t = &s[{range}]; let n = t.len(); println!(\"{{}}\", n); }}"
        );
        runs_with_exit(&source, "before\n", 101);
    }
    runs_with_exit(
        "fn main() { let a = [1i32, 2, 3, 4, 5]; let s: &[i32] = &a; let m = 0usize.wrapping_sub(1); println!(\"before\"); let t = &s[..=m]; let n = t.len(); println!(\"{}\", n); }",
        "before\n",
        101,
    );
}

#[test]
fn str_range_kinds_borrow_source_lifetime_and_check_char_boundaries() {
    // Expected stdout from stock rustc 1.98.1.
    runs_with_exit(
        r#"struct P<'a> { s: &'a str, pos: usize }
impl<'a> P<'a> {
    fn new(s: &'a str) -> Self { P { s, pos: 0 } }
    fn rest(&mut self) -> Option<&'a str> { self.pos = 7; Some(&self.s[self.pos..]) }
}
fn main() {
    let mut p = P::new("héllo wörld");
    while let Some(w) = p.rest() { println!("[{}]", w); break; }
}"#,
        "[wörld]\n",
        0,
    );
    runs_with_exit(
        r#"fn main() {
    let s = "héllo wörld";
    let a: &str = &s[0..3];
    let b: &str = &s[..1];
    let c: &str = &s[7..];
    let d: &str = &s[..];
    println!("[{}] [{}] [{}] [{}]", a, b, c, d);
}"#,
        "[hé] [h] [wörld] [héllo wörld]\n",
        0,
    );
    runs_with_exit(
        r#"fn main() {
    let s = "héllo wörld";
    let e: &str = &s[3..=5];
    let f: &str = &s[12..];
    let g: &str = &s[1..3];
    println!("[{}] [{}] {}", e, f, g.len());
}"#,
        "[llo] [d] 2\n",
        0,
    );
    // Non-boundary start, non-boundary end, end > len and start > end all
    // panic before later output (stock exit 101).
    for range in ["2..4", "0..2", "..99", "5..3", "9..", "..=13"] {
        let source = format!(
            "fn main() {{ let s = \"héllo wörld\"; println!(\"before\"); let t: &str = &s[{range}]; println!(\"{{}}\", t.len()); }}"
        );
        runs_with_exit(&source, "before\n", 101);
    }
}

#[test]
fn str_range_rejects_mutable_borrow_and_non_usize_bounds() {
    for body in [
        "let s: &str = \"abc\"; let t = &mut s[1..]; println!(\"{}\", t.len());",
        "let s: &str = \"abc\"; let i = 1i32; let t = &s[i..]; println!(\"{}\", t);",
    ] {
        let scratch = Scratch::new();
        let input = scratch.source(&format!("fn main() {{ {body} }}"));
        let output = scratch.path("program");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, 1, check), &scratch, 20);
            assert!(!result.status.success(), "{body} accepted");
            assert!(!output.exists());
        }
    }
}
