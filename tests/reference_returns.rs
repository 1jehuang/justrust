// B3a shared whole-parameter reference-return acceptance, frozen stock corpus.
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
                "jr-reference-returns-{}-{epoch}-{id}",
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
fn accepts_identity() {
    runs(
        r#"fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;*id(&x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_alpha_renamed() {
    runs(
        r#"fn id<'renamed>(x:&'renamed i32)->&'renamed i32{x} fn run()->i32{let x=7;*id(&x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_same_region_select() {
    runs(
        r#"fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let x=3;let y=7;*choose(&x,&y,false)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_independent_short_input() {
    runs(
        r#"fn choose<'a,'b>(x:&'a i32,y:&'b i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=3;r=choose(&x,&y);}*r}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_independent_different_pointee() {
    runs(
        r#"fn choose<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=choose(&x,&y);}*r}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_outlives_shortening() {
    runs(
        r#"fn short<'a:'b,'b>(x:&'a i32)->&'b i32{x} fn run()->i32{let x=7;*short(&x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_forward_return() {
    runs(
        r#"fn id<'a>(x:&'a i32)->&'a i32{x} fn forward<'a>(x:&'a i32)->&'a i32{id(x)} fn run()->i32{let x=7;*forward(&x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_recursive_return() {
    runs(
        r#"fn recur<'a>(x:&'a i32,n:i32)->&'a i32{if n==0{x}else{recur(x,n-1)}} fn run()->i32{let x=7;*recur(&x,3)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_last_use_then_mutation() {
    runs(
        r#"fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let mut x=3;let r=id(&x);let n=*r;x=7;n+x}
fn main() { if run() == 10 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_argument_carrier_storage_dead() {
    runs(
        r#"fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let a=&x;r=id(a);}*r}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_returned_shared_copy() {
    runs(
        r#"fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r=id(&x);let s=r;*r+*s}
fn main() { if run() == 14 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_whole_tuple_result() {
    runs(
        r#"fn id<'a>(x:&'a(i32,i32))->&'a(i32,i32){x} fn run()->i32{let x=(3,7);let r=id(&x);(*r).0*10+(*r).1}
fn main() { if run() == 37 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_whole_struct_result() {
    runs(
        r#"struct S{x:i32} fn id<'a>(x:&'a S)->&'a S{x} fn run()->i32{let x=S{x:7};(*id(&x)).x}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_full64_result_transport() {
    runs(
        r#"fn id<'a>(x:&'a usize)->&'a usize{x} fn run()->i32{let x=18446744073709551615usize;if *id(&x)==18446744073709551615usize{7}else{0}}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_private_explicit_reborrow() {
    runs(
        r#"fn id<'a>(x:&'a i32)->&'a i32{let r=&*x;r} fn run()->i32{let x=7;*id(&x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_forward_independent_dependency() {
    runs(
        r#"fn leaf<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{x} fn forward<'a,'b>(x:&'a i32,y:&'b bool)->&'a i32{leaf(x,y)} fn run()->i32{let x=7;let r:&i32;{let y=true;r=forward(&x,&y);}*r}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn accepts_branch_alias_last_use() {
    runs(
        r#"fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=7;let r=choose(&x,&y,true);let n=*r;x=4;y=8;n+x+y}
fn main() { if run() == 15 { println!("reference returns oracle ok"); } }
"#,
        "reference returns oracle ok\n",
    );
}

#[test]
fn rejects_invalid_callee_owned_return() {
    // Stock rejects this fixture with E0515.
    rejects(
        "callee_owned_return",
        r#"fn bad<'a>(x:&'a i32)->&'a i32{let y=7;&y} fn run()->i32{let x=3;*bad(&x)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unused_bad_callee() {
    // Stock rejects this fixture with E0515.
    rejects(
        "unused_bad_callee",
        r#"fn bad<'a>()->&'a i32{let y=7;&y} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unrelated_input_lifetime() {
    // Stock rejects this fixture for a missing lifetime outlives bound.
    rejects(
        "unrelated_input_lifetime",
        r#"fn bad<'a,'b>(x:&'a i32,y:&'b i32)->&'a i32{y} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_same_region_short_owner() {
    // Stock rejects this fixture with E0597.
    rejects(
        "same_region_short_owner",
        r#"fn choose<'a>(x:&'a i32,y:&'a i32)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=3;r=choose(&x,&y);}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_different_pointee_same_region() {
    // Stock rejects this fixture with E0597.
    rejects(
        "different_pointee_same_region",
        r#"fn choose<'a>(x:&'a i32,y:&'a bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=choose(&x,&y);}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owner_dies_before_result() {
    // Stock rejects this fixture with E0597.
    rejects(
        "owner_dies_before_result",
        r#"fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let r:&i32;{let x=7;r=id(&x);}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owner_mutation_before_result_use() {
    // Stock rejects this fixture with E0506.
    rejects(
        "owner_mutation_before_result_use",
        r#"fn id<'a>(x:&'a i32)->&'a i32{x} fn run()->i32{let mut x=3;let r=id(&x);x=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_branch_candidate_mutation() {
    // Stock rejects this fixture with E0506.
    rejects(
        "branch_candidate_mutation",
        r#"fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&'a i32{if b{x}else{y}} fn run()->i32{let mut x=3;let y=7;let r=choose(&x,&y,false);x=9;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_mutable_result() {
    // Stock accepts and runs this fixture, outside the bounded B3a slice.
    rejects(
        "mutable_result",
        r#"fn id<'a>(x:&'a mut i32)->&'a mut i32{x} fn run()->i32{let mut x=3;*id(&mut x)=7;x}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_projected_result() {
    // Stock accepts and runs this fixture, outside the bounded B3a slice.
    rejects(
        "projected_result",
        r#"struct S{x:i32} fn get<'a>(x:&'a S)->&'a i32{&x.x} fn run()->i32{let x=S{x:7};*get(&x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_elided_result() {
    // Stock accepts and runs this fixture, outside the bounded B3a slice.
    rejects(
        "elided_result",
        r#"fn id(x:&i32)->&i32{x} fn run()->i32{let x=7;*id(&x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_static_result() {
    // Stock accepts and runs this fixture, outside the bounded B3a slice.
    rejects(
        "static_result",
        r#"static X:i32=7;fn get()->&'static i32{&X} fn run()->i32{*get()}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_mutable_input_shared_result() {
    // Stock accepts and runs this fixture, outside the bounded B3a slice.
    rejects(
        "mutable_input_shared_result",
        r#"fn id<'a>(x:&'a mut i32)->&'a i32{&*x} fn run()->i32{let mut x=7;*id(&mut x)}
fn main() { if run() == 7 { println!("reference returns oracle ok"); } }
"#,
    );
}
