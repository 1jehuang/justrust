//! Typed deferred-local acceptance, independently stock-classified with rustc 1.98.1.
//! Edition 2024, 20 exact fixtures. No stock compiler is invoked by native tests.
//! Untyped, inferred-type and deferred destructuring cases are legal-unsupported boundaries.
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
            let path = base.join(format!("jr-deferred-{}-{epoch}-{id}", std::process::id()));
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
fn runs(source: &str, expected: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("program");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                result.status.success(),
                "workers={workers}, check={check}: {result:?}\n{}\n{source}",
                String::from_utf8_lossy(&result.stderr)
            );
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(&mut Command::new(output), &scratch, 5);
        assert!(
            result.status.success(),
            "runtime failure: {result:?}\n{source}"
        );
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        assert!(
            result.stderr.is_empty(),
            "unexpected runtime stderr: {result:?}"
        );
    }
}
fn rejects(label: &str, source: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: accepted workers={workers}, check={check}\n{source}"
            );
            assert!(
                result.status.code().is_some(),
                "{label}: compiler terminated by signal: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: no diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: compiler panic: {result:?}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn accepts_scalar_first_init() {
    runs(
        r#"fn run()->i32{let x:i32;x=7;x}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_tuple_first_init() {
    runs(
        r#"fn run()->i32{let x:(i32,bool);x=(7,true);if x.1{x.0}else{0}}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_struct_first_init() {
    runs(
        r#"struct S{n:i32} fn run()->i32{let x:S;x=S{n:7};x.n}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_enum_first_init() {
    runs(
        r#"enum E{A,B} fn run()->i32{let x:E;x=E::B;match x{E::A=>1,E::B=>7}}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_if_both_arms() {
    runs(
        r#"fn choose(b:bool)->i32{let x:i32;if b{x=3;}else{x=7;}x} fn run()->i32{choose(true)+choose(false)}
fn main() { if run() == 10 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_match_both_arms() {
    runs(
        r#"fn choose(b:bool)->i32{let x:i32;match b{true=>{x=3;},false=>{x=7;}}x} fn run()->i32{choose(true)+choose(false)}
fn main() { if run() == 10 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_mutable_moved_root_reinit() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let mut x:S;x=S{n:3};let a=take(x);x=S{n:7};a+take(x)}
fn main() { if run() == 10 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_lexical_shadow() {
    runs(
        r#"fn run()->i32{let x:i32;x=3;let y={let x:i32;x=7;x};x+y}
fn main() { if run() == 10 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_uninitialized() {
    runs(
        r#"enum E{A,B} fn run()->i32{let x:E;match x{_=>7}}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn accepts_singleton_uninitialized() {
    runs(
        r#"enum E{A} fn run()->i32{let x:E;match x{E::A=>7}}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
        "deferred oracle ok\n",
    );
}

#[test]
fn rejects_invalid_immutable_second_assignment() {
    // Stock rejects this fixture with E0384.
    rejects(
        "immutable_second_assignment",
        r#"fn run()->i32{let x:i32;x=3;x=7;x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_branch_missing_init() {
    // Stock rejects this fixture with E0381.
    rejects(
        "branch_missing_init",
        r#"fn choose(b:bool)->i32{let x:i32;if b{x=3;}x} fn run()->i32{choose(false)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_direct_uninitialized_read() {
    // Stock rejects this fixture with E0381.
    rejects(
        "direct_uninitialized_read",
        r#"fn run()->i32{let x:i32;x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_loop_immutable_assignment() {
    // Stock rejects this fixture with E0384.
    rejects(
        "loop_immutable_assignment",
        r#"fn run()->i32{let x:i32;let mut n=0;while n<2{x=n;n=n+1;}0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_projected_write_uninitialized() {
    // Stock rejects this fixture with E0381.
    rejects(
        "projected_write_uninitialized",
        r#"struct S{n:i32} fn run()->i32{let mut x:S;x.n=7;x.n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_variant_uninitialized() {
    // Stock rejects this fixture with E0381.
    rejects(
        "variant_uninitialized",
        r#"enum E{A,B} fn run()->i32{let x:E;match x{E::A=>1,E::B=>7}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_immutable_moved_root_reinit() {
    // Stock rejects this fixture with E0384.
    rejects(
        "immutable_moved_root_reinit",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let x:S;x=S{n:3};let a=take(x);x=S{n:7};a+take(x)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_untyped_deferred() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "untyped_deferred",
        r#"fn run()->i32{let x;x=7;x}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_backward_inferred_type() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "backward_inferred_type",
        r#"fn run()->i32{let x:_;x=7;x}
fn main() { if run() == 7 { println!("deferred oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_deferred_tuple_pattern() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "deferred_tuple_pattern",
        r#"fn run()->i32{let (a,b):(i32,i32);(a,b)=(3,7);a+b}
fn main() { if run() == 10 { println!("deferred oracle ok"); } }
"#,
    );
}
