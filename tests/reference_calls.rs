// B2 noescape reference-call acceptance. Stock-classified corpus is frozen.
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
                "jr-reference-calls-{}-{epoch}-{id}",
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
fn accepts_shared_reference_parameter() {
    runs(
        r#"fn read(r:&i32)->i32{*r} fn run()->i32{let x=7;read(&x)}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_immutable_mutable_parameter_carrier() {
    runs(
        r#"fn write(r:&mut i32){*r=7;} fn run()->i32{let mut x=3;write(&mut x);x}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_repeated_calls_implicit_reborrow() {
    runs(
        r#"fn bump(r:&mut i32){*r=*r+2;} fn run()->i32{let mut x=3;let r=&mut x;bump(r);bump(r);*r}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_shared_alias_arguments() {
    runs(
        r#"fn sum(a:&i32,b:&i32)->i32{*a+*b} fn run()->i32{let x=7;let r=&x;sum(r,r)}
fn main() { if run() == 14 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_disjoint_mutable_field_arguments() {
    runs(
        r#"struct S{a:i32,b:i32} fn change(a:&mut i32,b:&mut i32){*a=*a+1;*b=*b+2;} fn run()->i32{let mut s=S{a:3,b:7};change(&mut s.a,&mut s.b);s.a+s.b}
fn main() { if run() == 13 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_explicit_call_reborrow() {
    runs(
        r#"fn bump(r:&mut i32){*r=*r+4;} fn run()->i32{let mut x=3;let r=&mut x;bump(&mut *r);*r}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_formal_mutable_handle_move() {
    runs(
        r#"fn bump(r:&mut i32){let q=r;*q=*q+4;} fn run()->i32{let mut x=3;let r=&mut x;bump(r);*r}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_formal_handle_rebinding() {
    runs(
        r#"fn change(p:&mut i32)->i32{let mut r=p;let mut y=7;r=&mut y;*r} fn run()->i32{let mut x=3;let n=change(&mut x);x+n}
fn main() { if run() == 10 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_forward_reference_parameter() {
    runs(
        r#"fn leaf(r:&mut i32){*r=7;} fn forward(r:&mut i32){leaf(r);} fn run()->i32{let mut x=3;forward(&mut x);x}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_nested_mutator_calls() {
    runs(
        r#"fn leaf(r:&mut i32){*r=*r+2;} fn outer(r:&mut i32){leaf(r);leaf(r);} fn run()->i32{let mut x=3;outer(&mut x);x}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_recursive_reference_mutation() {
    runs(
        r#"fn bump(r:&mut i32,n:i32){if n>0{*r=*r+1;bump(r,n-1);}} fn run()->i32{let mut x=3;bump(&mut x,4);x}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_forward_shared_parameter() {
    runs(
        r#"fn leaf(r:&i32)->i32{*r} fn forward(r:&i32)->i32{leaf(r)} fn run()->i32{let x=7;forward(&x)}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_typed_local_expected_reborrow() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let q:&mut i32=r;*q=7;*r}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_assignment_expected_reborrow() {
    runs(
        r#"fn run()->i32{let mut x=3;let r=&mut x;let q:&mut i32;q=r;*q=7;*r}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_mutable_to_shared_call_reborrow() {
    runs(
        r#"fn sum(a:&i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;let r=&mut x;let n=sum(r,*r);*r=7;n+*r}
fn main() { if run() == 13 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_noescape_call_loan_ends() {
    runs(
        r#"fn read(r:&i32)->i32{*r} fn run()->i32{let mut x=3;let n=read(&x);x=7;n+x}
fn main() { if run() == 10 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_copy_aggregate_reference_parameter() {
    runs(
        r#"fn sum(r:&(i32,i32))->i32{(*r).0*10+(*r).1} fn run()->i32{let p=(3,7);sum(&p)}
fn main() { if run() == 37 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_full64_usize_reference_parameter() {
    runs(
        r#"fn read(r:&usize)->usize{*r} fn run()->i32{let x:usize=18446744073709551615usize;if read(&x)==18446744073709551615usize{7}else{0}}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn rejects_invalid_mutable_alias_arguments() {
    // Stock rejects this fixture with E0499.
    rejects(
        "mutable_alias_arguments",
        r#"fn sum(a:&mut i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut x=7;sum(&mut x,&mut x)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_shared_mutable_alias_arguments() {
    // Stock rejects this fixture with E0502.
    rejects(
        "shared_mutable_alias_arguments",
        r#"fn sum(a:&i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut x=7;sum(&x,&mut x)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_shared_parameter_cannot_write() {
    // Stock rejects this fixture with E0594.
    rejects(
        "shared_parameter_cannot_write",
        r#"fn write(r:&i32){*r=7;} fn run()->i32{let x=3;write(&x);x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_formal_handle_used_after_move() {
    // Stock rejects this fixture with E0382.
    rejects(
        "formal_handle_used_after_move",
        r#"fn read(r:&mut i32)->i32{let q=r;*q+*r} fn run()->i32{let mut x=7;read(&mut x)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_first_argument_loan_later_mutation() {
    // Stock rejects this fixture with E0506.
    rejects(
        "first_argument_loan_later_mutation",
        r#"fn sum(a:&i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;sum(&x,{x=7;0})}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_first_mutable_argument_later_owner_read() {
    // Stock rejects this fixture with E0503.
    rejects(
        "first_mutable_argument_later_owner_read",
        r#"fn sum(a:&mut i32,b:i32)->i32{*a+b} fn run()->i32{let mut x=3;sum(&mut x,x)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mutable_parameter_immutable_owner() {
    // Stock rejects this fixture with E0596.
    rejects(
        "mutable_parameter_immutable_owner",
        r#"fn write(r:&mut i32){*r=7;} fn run()->i32{let x=3;write(&mut x);x}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mutable_array_index_arguments_overlap() {
    // Stock rejects this fixture with E0499.
    rejects(
        "mutable_array_index_arguments_overlap",
        r#"fn sum(a:&mut i32,b:&mut i32)->i32{*a+*b} fn run()->i32{let mut a=[3,7];sum(&mut a[0],&mut a[1])}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_reference_return_escape() {
    runs(
        r#"fn pass(r:&i32)->&i32{r} fn run()->i32{let x=7;*pass(&x)}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_reference_aggregate_field() {
    #[path = "support/closure_oracle.rs"]
    mod oracle;
    oracle::check(
        "reference_aggregate_field",
        &[oracle::Case {
            name: "reference_aggregate_field",
            source: r#"struct R<'a>{r:&'a i32} fn read(r:&i32)->i32{*r} fn run()->i32{let x=7;let r=R{r:&x};read(r.r)}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
            expect: oracle::Expect::Run("borrow params oracle ok\n"),
        }],
    );
}

#[test]
fn accepts_reference_to_reference_parameter() {
    // Unchanged source independently passes stock and native workers 1/8.
    runs(
        r#"fn read(r:&&i32)->i32{**r} fn run()->i32{let x=7;let r=&x;read(&r)}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn accepts_rvalue_borrow_argument() {
    runs(
        r#"fn read(r:&i32)->i32{*r} fn run()->i32{read(&7)}
fn main() { if run() == 7 { println!("borrow params oracle ok"); } }
"#,
        "borrow params oracle ok\n",
    );
}

#[test]
fn projected_mutable_argument_reborrows_match_stock() {
    #[path = "support/closure_oracle.rs"]
    mod oracle;
    oracle::check(
        "projected_mutable_reborrow",
        &[
            oracle::Case {
                name: "reborrow_does_not_move",
                source: "fn put(r:&mut i32){*r+=1;} fn relay(r:&mut &mut i32){put(*r);put(*r);} fn main(){let mut x=5;let mut r=&mut x;relay(&mut r);println!(\"{}\",x);}",
                expect: oracle::Expect::Run("7\n"),
            },
            oracle::Case {
                name: "shared_reference_cannot_authorize_mutation",
                source: "fn put(r:&mut i32){*r=1;} fn bad(r:&&mut i32){put(*r);} fn main(){}",
                expect: oracle::Expect::Invalid("E0596"),
            },
        ],
    );
}
