// B3b mutable whole-parameter reference returns, exact frozen stock corpus.
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
                "jr-mutable-reference-returns-{}-{epoch}-{id}",
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
fn accepts_identity_scalar_write() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_full64_storage_write() {
    runs(
        r#"fn id<'a>(r:&'a mut usize)->&'a mut usize{r} fn run()->i32{let mut x=3usize;let q=id(&mut x);*q=18446744073709551615usize;if x==18446744073709551615usize{7}else{0}}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_aggregate_storage_write() {
    runs(
        r#"struct S{x:i32,y:i32} fn id<'a>(r:&'a mut S)->&'a mut S{r} fn run()->i32{let mut s=S{x:3,y:4};let q=id(&mut s);(*q).x=7;(*q).y=8;s.x+s.y}
fn main() { if run() == 15 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_two_candidates_both_branches() {
    runs(
        r#"fn choose<'a>(x:&'a mut i32,y:&'a mut i32,b:bool)->&'a mut i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=4;let q=choose(&mut x,&mut y,true);*q=7;let r=choose(&mut x,&mut y,false);*r=8;x+y}
fn main() { if run() == 15 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_original_parameter_index() {
    runs(
        r#"fn pick<'a>(n:i32,x:&'a mut i32,y:&'a mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let mut y=true;let q=pick(0,&mut x,&mut y);*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_expected_return_reborrow() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{let q:&mut i32=r;q} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_explicit_return_reborrow() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{&mut *r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_forward_recursive() {
    runs(
        r#"fn recur<'a>(r:&'a mut i32,n:i32)->&'a mut i32{if n==0{r}else{recur(r,n-1)}} fn forward<'a>(r:&'a mut i32)->&'a mut i32{recur(r,3)} fn run()->i32{let mut x=3;let q=forward(&mut x);*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_result_handle_move() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let moved=q;*moved=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_parent_resumes_after_last_use() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);*q=7;*r=*r+1;*r}
fn main() { if run() == 8 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_shared_child_reborrow() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let child=&*q;let n=*child;*q=7;n+*q}
fn main() { if run() == 10 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_mutable_child_reborrow() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let child=&mut *q;*child=7;*q=*q+1;*q}
fn main() { if run() == 8 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_parent_carrier_storage_dead() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q:&mut i32;{let r=&mut x;q=id(r);}*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_independent_short_input() {
    runs(
        r#"fn pick<'a,'b>(x:&'a mut i32,y:&'b mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let q:&mut i32;{let mut y=true;q=pick(&mut x,&mut y);}*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_mutate_nonreturned_input() {
    runs(
        r#"fn pick<'a,'b>(x:&'a mut i32,y:&'b mut i32)->&'a mut i32{*y=8;x} fn run()->i32{let mut x=3;let mut y=4;let q=pick(&mut x,&mut y);*q=7;x+y}
fn main() { if run() == 15 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn accepts_direct_call_copy_read() {
    runs(
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=7;*id(&mut x)}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn rejects_invalid_overlapping_mutable_actuals() {
    // Stock rejects this fixture with E0499.
    rejects(
        "overlapping_mutable_actuals",
        r#"fn choose<'a>(x:&'a mut i32,y:&'a mut i32)->&'a mut i32{x} fn run()->i32{let mut x=3;let q=choose(&mut x,&mut x);*q}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_result_used_after_move() {
    // Stock rejects this fixture with E0382.
    rejects(
        "result_used_after_move",
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);let moved=q;*moved=7;*q}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_live_child_parent_read() {
    // Stock rejects this fixture with E0503.
    rejects(
        "live_child_parent_read",
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);let n=*r;*q+n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_live_child_parent_write() {
    // Stock rejects this fixture with E0506.
    rejects(
        "live_child_parent_write",
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);*r=7;*q}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_live_child_parent_move() {
    // Stock rejects this fixture with E0505.
    rejects(
        "live_child_parent_move",
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(&mut *r);let moved=r;*q+*moved}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_live_child_parent_borrow() {
    // Stock rejects this fixture with E0499.
    rejects(
        "live_child_parent_borrow",
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;let r=&mut x;let q=id(r);let s=&mut *r;*q+*s}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_owned_local_return() {
    // Stock rejects this fixture with E0515.
    rejects(
        "owned_local_return",
        r#"fn bad<'a>(r:&'a mut i32)->&'a mut i32{let mut y=7;&mut y} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unused_bad_callee() {
    // Stock rejects this fixture with E0515.
    rejects(
        "unused_bad_callee",
        r#"fn bad<'a>()->&'a mut i32{let mut y=7;&mut y} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_missing_universal_outlives() {
    // Stock rejects this fixture for a missing lifetime outlives bound.
    rejects(
        "missing_universal_outlives",
        r#"fn bad<'a,'b>(x:&'a mut i32,y:&'b mut i32)->&'a mut i32{y} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_same_region_different_pointee_dependency() {
    // Stock rejects this fixture with E0597.
    rejects(
        "same_region_different_pointee_dependency",
        r#"fn pick<'a>(x:&'a mut i32,y:&'a mut bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let q:&mut i32;{let mut y=true;q=pick(&mut x,&mut y);}*q}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_projected_mutable_result() {
    // Stock accepts and runs this fixture, outside the bounded B3b slice.
    rejects(
        "projected_mutable_result",
        r#"struct S{x:i32} fn get<'a>(x:&'a mut S)->&'a mut i32{&mut x.x} fn run()->i32{let mut s=S{x:3};let q=get(&mut s);*q=7;s.x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_shared_input_mutable_result() {
    // Stock accepts and runs this fixture, outside the bounded B3b slice.
    rejects(
        "shared_input_mutable_result",
        r#"fn pick<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let q=pick(&mut x,&y);*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
    );
}

#[test]
fn accepts_elided_mutable_result() {
    runs(
        r#"fn id(r:&mut i32)->&mut i32{r} fn run()->i32{let mut x=3;let q=id(&mut x);*q=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
        "mutable returns oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_computed_call_assignment() {
    // Stock accepts and runs this fixture, outside the bounded B3b slice.
    rejects(
        "computed_call_assignment",
        r#"fn id<'a>(r:&'a mut i32)->&'a mut i32{r} fn run()->i32{let mut x=3;*id(&mut x)=7;x}
fn main() { if run() == 7 { println!("mutable returns oracle ok"); } }
"#,
    );
}
