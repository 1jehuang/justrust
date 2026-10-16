// E2 builtin reference-field access, exact frozen stock twenty-case corpus.
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
                "jr-reference-fields-{}-{epoch}-{id}",
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
fn accepts_shared_struct_field_read() {
    runs(
        r#"struct S{x:i32} fn run()->i32{let s=S{x:7};let r=&s;r.x}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_shared_tuple_field_read() {
    runs(
        r#"fn run()->i32{let p=(3,7);let r=&p;r.0*10+r.1}
fn main() { if run() == 37 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_mutable_struct_tuple_writes() {
    runs(
        r#"struct S{x:i32} fn run()->i32{let mut s=S{x:3};let mut p=(3,4);let r=&mut s;let q=&mut p;r.x=7;q.1=8;s.x+p.1}
fn main() { if run() == 15 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_shared_field_borrow_return() {
    runs(
        r#"struct S{x:i32} fn get(r:&S)->&i32{&r.x} fn run()->i32{let s=S{x:7};*get(&s)}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_mutable_field_borrow_return() {
    runs(
        r#"struct S{x:i32} fn get(r:&mut S)->&mut i32{&mut r.x} fn run()->i32{let mut s=S{x:3};let q=get(&mut s);*q=7;s.x}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_nested_fields_static_array() {
    runs(
        r#"struct Inner{p:[i32;2]} struct Outer{inner:Inner} fn run()->i32{let mut s=Outer{inner:Inner{p:[3,4]}};let r=&mut s;r.inner.p[1]=7;r.inner.p[1]}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_genuine_reference_call_field_reads() {
    runs(
        r#"struct S{x:i32} fn shared(s:&S)->&S{s} fn unique(s:&mut S)->&mut S{s} fn run()->i32{let s=S{x:3};let mut t=S{x:7};shared(&s).x*10+unique(&mut t).x}
fn main() { if run() == 37 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_explicit_deref_parity() {
    runs(
        r#"struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;r.x=7;if r.x==(*r).x{r.x}else{0}}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_disjoint_reference_field_borrows() {
    runs(
        r#"struct S{x:i32,y:i32} fn run()->i32{let mut s=S{x:3,y:4};let r=&mut s;let a=&mut r.x;let b=&mut r.y;*a=7;*b=8;r.x+r.y}
fn main() { if run() == 15 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn accepts_field_loan_lastuse_resume() {
    runs(
        r#"struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;let q=&r.x;let n=*q;r.x=7;n+r.x}
fn main() { if run() == 10 { println!("reference fields oracle ok"); } }
"#,
        "reference fields oracle ok\n",
    );
}

#[test]
fn rejects_invalid_noncopy_field_move() {
    // Stock rejects this fixture with E0507.
    rejects(
        "noncopy_field_move",
        r#"struct N{x:i32} struct S{n:N} fn run()->i32{let s=S{n:N{x:7}};let r=&s;let n=r.n;n.x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_shared_reference_field_write() {
    // Stock rejects this fixture with E0594.
    rejects(
        "shared_reference_field_write",
        r#"struct S{x:i32} fn run()->i32{let s=S{x:3};let r=&s;r.x=7;r.x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_private_reference_field() {
    // Stock rejects this fixture with E0616.
    rejects(
        "private_reference_field",
        r#"mod m{pub struct S{x:i32} pub fn make()->S{S{x:7}}} fn run()->i32{let s=m::make();let r=&s;r.x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_local_field_return() {
    // Stock rejects this fixture with E0515.
    rejects(
        "local_field_return",
        r#"struct S{x:i32} fn bad(r:&S)->&i32{let s=S{x:7};&s.x} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_field_result_owner_storage_dead() {
    // Stock rejects this fixture with E0597.
    rejects(
        "field_result_owner_storage_dead",
        r#"struct S{x:i32} fn get(r:&S)->&i32{&r.x} fn run()->i32{let q:&i32;{let s=S{x:7};q=get(&s);}*q}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_field_write_while_child_live() {
    // Stock rejects this fixture with E0506.
    rejects(
        "field_write_while_child_live",
        r#"struct S{x:i32} fn run()->i32{let mut s=S{x:3};let r=&mut s;let q=&mut r.x;r.x=7;*q}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_computed_call_field_assignment() {
    // Stock accepts and runs this fixture, outside the bounded E2 slice.
    rejects(
        "computed_call_field_assignment",
        r#"struct S{x:i32} fn get(r:&mut S)->&mut S{r} fn run()->i32{let mut s=S{x:3};get(&mut s).x=7;s.x}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_borrow_call_field_rvalue() {
    // Stock accepts and runs this fixture, outside the bounded E2 slice.
    rejects(
        "borrow_call_field_rvalue",
        r#"struct S{x:i32} fn get(r:&S)->&S{r} fn run()->i32{let s=S{x:7};let q=&get(&s).x;*q}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_bare_reference_index() {
    // Stock accepts and runs this fixture, outside the bounded E2 slice.
    rejects(
        "bare_reference_index",
        r#"fn run()->i32{let a=[3,7];let r=&a;r[1]}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_user_method_autoderef() {
    // Stock accepts and runs this fixture, outside the bounded E2 slice.
    rejects(
        "user_method_autoderef",
        r#"struct S{x:i32} impl S{fn value(&self)->i32{self.x}} fn run()->i32{let s=S{x:7};let r=&s;r.value()}
fn main() { if run() == 7 { println!("reference fields oracle ok"); } }
"#,
    );
}
