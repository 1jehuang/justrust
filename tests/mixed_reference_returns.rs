// E3 mixed-capability reference results, exact frozen stock twenty-case corpus.
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
                "jr-mixed-reference-returns-{}-{epoch}-{id}",
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
fn accepts_shared_result_from_mutable_input() {
    runs(
        r#"fn get(x:&mut i32)->&i32{&*x} fn run()->i32{let mut x=7;*get(&mut x)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_shared_target_unrelated_mutable_input() {
    runs(
        r#"fn get<'a,'b>(x:&'a i32,y:&'b mut bool)->&'a i32{x} fn run()->i32{let x=7;let mut y=true;*get(&x,&mut y)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_mutable_target_shared_result_mixed_inputs() {
    runs(
        r#"fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=7;let y=true;*get(&mut x,&y)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_shared_projected_from_mutable() {
    runs(
        r#"fn get(x:&mut(i32,i32))->&i32{&x.1} fn run()->i32{let mut x=(3,7);*get(&mut x)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_mutable_result_unrelated_shared_input() {
    runs(
        r#"fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let r=get(&mut x,&y);*r=7;x}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_mutable_projected_mixed_inputs() {
    runs(
        r#"struct S{x:i32} fn get<'a,'b>(x:&'a mut S,y:&'b bool)->&'a mut i32{&mut x.x} fn run()->i32{let mut x=S{x:3};let y=true;let r=get(&mut x,&y);*r=7;x.x}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_independent_short_mutable_dependency() {
    runs(
        r#"fn get<'a,'b>(x:&'a i32,y:&'b mut bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let mut y=true;r=get(&x,&mut y);}*r}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_independent_short_shared_dependency() {
    runs(
        r#"fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let y=true;r=get(&mut x,&y);}*r=7;x}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_shared_result_mixed_candidate_branch() {
    runs(
        r#"fn get<'a>(x:&'a i32,y:&'a mut i32,b:bool)->&'a i32{if b{x}else{&*y}} fn run()->i32{let x=3;let mut y=7;*get(&x,&mut y,false)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_recursive_mixed_shared_return() {
    runs(
        r#"fn recur<'a,'b>(x:&'a mut i32,y:&'b bool,n:i32)->&'a i32{if n==0{&*x}else{recur(x,y,n-1)}} fn run()->i32{let mut x=7;let y=true;*recur(&mut x,&y,3)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_shared_child_parent_read_lastuse_resume() {
    runs(
        r#"fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);let n=*r+*parent;*parent=7;n+*parent}
fn main() { if run() == 13 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn accepts_mutate_unrelated_nonreturned_input() {
    runs(
        r#"fn get<'a,'b>(x:&'a i32,y:&'b mut i32)->&'a i32{*y=8;x} fn run()->i32{let x=7;let mut y=3;let r=get(&x,&mut y);*r+y}
fn main() { if run() == 15 { println!("mixed returns oracle ok"); } }
"#,
        "mixed returns oracle ok\n",
    );
}

#[test]
fn rejects_invalid_shared_input_cannot_upgrade_mutable() {
    // Stock rejects this fixture with E0596.
    rejects(
        "shared_input_cannot_upgrade_mutable",
        r#"fn bad<'a>(x:&'a i32)->&'a mut i32{&mut *x} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_overlapping_mixed_actuals() {
    // Stock rejects this fixture with E0502.
    rejects(
        "overlapping_mixed_actuals",
        r#"fn get<'a>(x:&'a mut i32,y:&'a i32)->&'a i32{&*x} fn run()->i32{let mut x=3;*get(&mut x,&x)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_shared_result_same_region_other_pointee_pin() {
    // Stock rejects this fixture with E0597.
    rejects(
        "shared_result_same_region_other_pointee_pin",
        r#"fn get<'a>(x:&'a mut i32,y:&'a bool)->&'a i32{&*x} fn run()->i32{let mut x=7;let r:&i32;{let y=true;r=get(&mut x,&y);}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mutable_result_same_region_shared_pin() {
    // Stock rejects this fixture with E0597.
    rejects(
        "mutable_result_same_region_shared_pin",
        r#"fn get<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let y=true;r=get(&mut x,&y);}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_live_shared_result_parent_write() {
    // Stock rejects this fixture with E0506.
    rejects(
        "live_shared_result_parent_write",
        r#"fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a i32{&*x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);*parent=7;*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_live_mutable_result_parent_read() {
    // Stock rejects this fixture with E0503.
    rejects(
        "live_mutable_result_parent_read",
        r#"fn get<'a,'b>(x:&'a mut i32,y:&'b bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let parent=&mut x;let r=get(parent,&y);let n=*parent;*r+n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_explicit_mutable_borrow_to_shared_result() {
    // Stock accepts and runs this fixture, outside the bounded E3 slice.
    rejects(
        "explicit_mutable_borrow_to_shared_result",
        r#"fn get(x:&mut i32)->&i32{&mut *x} fn run()->i32{let mut x=7;*get(&mut x)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_projected_mutable_borrow_to_shared_result() {
    // Stock accepts and runs this fixture, outside the bounded E3 slice.
    rejects(
        "projected_mutable_borrow_to_shared_result",
        r#"struct S{x:i32} fn get(x:&mut S)->&i32{&mut x.x} fn run()->i32{let mut x=S{x:7};*get(&mut x)}
fn main() { if run() == 7 { println!("mixed returns oracle ok"); } }
"#,
    );
}
