//! B1 genuine nonescaping local references and borrowing acceptance.
//! All 35 exact fixtures were stock-classified with rustc 1.98.1, edition 2024.
//! Stock is an external scratch oracle only and is never invoked by these tests.
//! References in signatures/aggregates, reference referents and rvalue borrows remain boundaries.
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
            let path = base.join(format!("jr-borrowing-{}-{epoch}-{id}", std::process::id()));
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
fn accepts_shared_dereference() {
    runs(
        r#"fn run()->i32{let x=7;let r:&i32=&x;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_reference_copy() {
    runs(
        r#"fn run()->i32{let x=7;let r=&x;let s=r;*r+*s}
fn main() { if run() == 14 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_immutable_carrier_mutable_referent() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;*r=*r+4;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_nll_last_use() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&x;let n=*r;x=7;n+x}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_mutable_nll_last_use() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;*r=7;x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_mutable_reference_move() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_explicit_mutable_reborrow() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*s=7;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_explicit_shared_reborrow() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;let n=*s;*r=7;n+*r}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_disjoint_struct_fields() {
    runs(
        r#"struct S{a:i32,b:i32} fn run()->i32{let mut p=S{a:3,b:7};let a=&mut p.a;let b=&mut p.b;*a=*a+1;*a+*b}
fn main() { if run() == 11 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_branch_reference_origins() {
    runs(
        r#"fn choose(b:bool)->i32{let x=3;let y=7;let r:&i32;if b{r=&x;}else{r=&y;}*r} fn run()->i32{choose(true)+choose(false)}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_loop_last_use() {
    runs(
        r#"fn run()->i32{let mut x=0;let mut i=0;while i<3{let r=&mut x;*r=*r+1;i=i+1;}x}
fn main() { if run() == 3 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_array_elements() {
    runs(
        r#"fn run()->i32{let a=[3,7];let r=&a[0];let s=&a[1];*r+*s}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_copy_aggregate_dereference() {
    runs(
        r#"fn run()->i32{let p=(3,7);let r=&p;let q=*r;q.0*10+q.1}
fn main() { if run() == 37 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_lexical_mutable_loan_scope() {
    runs(
        r#"fn run()->i32{let mut x=3;{let r=&mut x;*r=7;}x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_invalid_mutable_carrier_shared_cannot_write() {
    // Stock rejects this fixture with E0594.
    rejects(
        "mutable_carrier_shared_cannot_write",
        r#"fn run()->i32{let mut x=3;let mut r=&x;*r=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mutable_borrow_immutable_owner() {
    // Stock rejects this fixture with E0596.
    rejects(
        "mutable_borrow_immutable_owner",
        r#"fn run()->i32{let x=3;let r=&mut x;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_shared_mutable_conflict() {
    // Stock rejects this fixture with E0502.
    rejects(
        "shared_mutable_conflict",
        r#"fn run()->i32{let mut x=3;let r=&x;let s=&mut x;*r+*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_two_mutable_loans() {
    // Stock rejects this fixture with E0499.
    rejects(
        "two_mutable_loans",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&mut x;*r+*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owner_write_while_loan_live() {
    // Stock rejects this fixture with E0506.
    rejects(
        "owner_write_while_loan_live",
        r#"fn run()->i32{let mut x=3;let r=&x;x=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owner_move_while_loan_live() {
    // Stock rejects this fixture with E0505.
    rejects(
        "owner_move_while_loan_live",
        r#"struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=x;(*r).n+y.n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_noncopy_move_through_shared() {
    // Stock rejects this fixture with E0507.
    rejects(
        "noncopy_move_through_shared",
        r#"struct S{n:i32} fn run()->i32{let x=S{n:7};let r=&x;let y=*r;y.n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mutable_reference_moved_carrier() {
    // Stock rejects this fixture with E0382.
    rejects(
        "mutable_reference_moved_carrier",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=r;*s=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_array_index_loans_overlap() {
    // Stock rejects this fixture with E0499.
    rejects(
        "array_index_loans_overlap",
        r#"fn run()->i32{let mut a=[3,7];let r=&mut a[0];let s=&mut a[1];*r+*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_referent_storage_dead() {
    // Stock rejects this fixture with E0597.
    rejects(
        "referent_storage_dead",
        r#"fn run()->i32{let r:&i32;{let x=7;r=&x;}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_parent_access_while_child_live() {
    // Stock rejects this fixture with E0506.
    rejects(
        "parent_access_while_child_live",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&mut *r;*r=7;*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_loop_referent_storage_dead() {
    // Stock rejects this fixture with E0597.
    rejects(
        "loop_referent_storage_dead",
        r#"fn run()->i32{let x=0;let mut r:&i32=&x;let mut i=0;while i<2{let y=i;r=&y;i=i+1;}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_reference_parameter_result() {
    runs(
        r#"fn pass(r:&i32)->&i32{r} fn run()->i32{let x=7;*pass(&x)}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_reference_aggregate_field() {
    // Stock accepts and runs this fixture, outside this bounded B1 slice.
    rejects(
        "reference_aggregate_field",
        r#"struct R<'a>{r:&'a i32} fn run()->i32{let x=7;let r=R{r:&x};*r.r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
    );
}

#[test]
fn accepts_reference_to_reference() {
    // Unchanged source independently passes stock and native workers 1/8.
    runs(
        r#"fn run()->i32{let x=7;let r=&x;let rr=&r;**rr}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_rvalue_borrow() {
    // Stock accepts and runs this fixture, outside this bounded B1 slice.
    rejects(
        "rvalue_borrow",
        r#"fn run()->i32{let r=&7;*r}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
    );
}

#[test]
fn accepts_parent_handle_dies_child_lives() {
    runs(
        r#"fn run()->i32{let mut x=3;let s:&mut i32;{let r=&mut x;s=&mut *r;}*s=7;x}
fn main() { if run() == 7 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn accepts_shared_carrier_reassignment() {
    runs(
        r#"fn run()->i32{let x=3;let y=7;let mut r=&x;let n=*r;r=&y;n+*r}
fn main() { if run() == 10 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}

#[test]
fn rejects_invalid_shared_copy_survives_carrier_overwrite() {
    // Stock rejects this fixture with E0506.
    rejects(
        "shared_copy_survives_carrier_overwrite",
        r#"fn run()->i32{let mut x=3;let y=7;let mut r=&x;let s=r;r=&y;x=9;*s+*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_shared_child_freezes_parent_write() {
    // Stock rejects this fixture with E0506.
    rejects(
        "shared_child_freezes_parent_write",
        r#"fn run()->i32{let mut x=3;let r=&mut x;let s=&*r;*r=7;*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_deref_lhs_resolved_after_rhs() {
    runs(
        r#"fn run()->i32{let mut x=1;let mut y=2;let mut r=&mut x;*r={r=&mut y;3};let w:usize=4294967296;let p=&w;if *p==4294967296{x+y}else{0}}
fn main() { if run() == 4 { println!("borrow oracle ok"); } }
"#,
        "borrow oracle ok\n",
    );
}
