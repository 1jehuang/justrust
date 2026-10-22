//! Fixed arrays with literal indices, a bounded native acceptance milestone.
//! All 30 exact fixtures were stock-classified with rustc 1.98.1, edition 2024.
//! Stock is an external scratch oracle only and is never invoked by these tests.
//! Non-Copy array-pattern bindings, slices, len and later-use empty inference are boundaries.
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
            let path = base.join(format!("jr-arrays-{}-{epoch}-{id}", std::process::id()));
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
fn accepts_array_literal() {
    runs(
        r#"fn run()->i32{let a=[3,7];a[0]*10+a[1]}
fn main() { if run() == 37 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_copy_repeat() {
    runs(
        r#"fn run()->i32{let a=[7;3];a[0]+a[1]+a[2]}
fn main() { if run() == 21 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_typed_empty_literal() {
    runs(
        r#"struct R{a:[i32;0]} enum E{A([i32;0])} fn make()->[i32;0]{[]} fn returned()->[i32;0]{return [];} fn take(a:[i32;0])->i32{1} fn run()->i32{let a:[i32;0]=[];let r=R{a:[]};let e=E::A([]);take(a)+take([])+take(make())+take(returned())+take(r.a)+match e{E::A(a)=>take(a)}}
fn main() { if run() == 6 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_zero_copy_whole() {
    runs(
        r#"fn consume(a:[i32;0])->i32{3} fn run()->i32{let a=[7;0];consume(a)+consume(a)}
fn main() { if run() == 6 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_repeat_zero_effect() {
    runs(
        r#"struct S{n:i32} fn make()->S{println!("once");S{n:3}} fn run()->i32{let a=[make();0];7}
fn main() { if run() == 7 { println!("array oracle ok"); } }
"#,
        "once\narray oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_repeat_one_effect() {
    runs(
        r#"struct S{n:i32} fn make()->S{println!("once");S{n:7}} fn run()->i32{let a=[make();1];a[0].n}
fn main() { if run() == 7 { println!("array oracle ok"); } }
"#,
        "once\narray oracle ok\n",
    );
}

#[test]
fn accepts_copy_field_noncopy_element() {
    runs(
        r#"struct S{n:i32} fn run()->i32{let a=[S{n:3},S{n:7}];let _=a[0];a[0].n+a[1].n+a[0].n}
fn main() { if run() == 13 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_mutable_scalar_index_assignment() {
    runs(
        r#"fn run()->i32{let mut a=[3,4];a[1]=7;a[0]*10+a[1]}
fn main() { if run() == 37 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_mutable_noncopy_index_assignment() {
    runs(
        r#"struct S{n:i32} fn run()->i32{let mut a=[S{n:3},S{n:4}];a[1]=S{n:7};a[0].n*10+a[1].n}
fn main() { if run() == 37 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn rejects_invalid_wildcard_uninitialized_index() {
    // Stock rejects this fixture with E0381.
    rejects(
        "wildcard_uninitialized_index",
        r#"struct S{n:i32} fn run()->i32{let a:[S;2];let _=a[0];7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_wildcard_moved_index() {
    // Stock rejects this fixture with E0382.
    rejects(
        "wildcard_moved_index",
        r#"struct S{n:i32} fn consume(a:[S;2])->i32{3} fn run()->i32{let a=[S{n:1},S{n:2}];let n=consume(a);let _=a[0];n+4}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_nested_array_values() {
    runs(
        r#"fn run()->i32{let a:[i32;2]=[3,7];let b:[[i32;2];2]=[a;2];a[0]*10+b[1][1]}
fn main() { if run() == 37 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_literal_source_order() {
    runs(
        r#"fn first()->i32{println!("first");3} fn second()->i32{println!("second");7} fn run()->i32{let a=[first(),second()];a[0]*10+a[1]}
fn main() { if run() == 37 { println!("array oracle ok"); } }
"#,
        "first\nsecond\narray oracle ok\n",
    );
}

#[test]
fn rejects_invalid_noncopy_index_move() {
    // Stock rejects this fixture with E0508.
    rejects(
        "noncopy_index_move",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let a=[S{n:7}];take(a[0])}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_noncopy_repeat_two() {
    // Stock rejects this fixture with E0277.
    rejects(
        "noncopy_repeat_two",
        r#"struct S{n:i32} fn run()->i32{let s=S{n:7};let a=[s;2];0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_repeat_zero_consumes() {
    // Stock rejects this fixture with E0382.
    rejects(
        "repeat_zero_consumes",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let s=S{n:7};let a=[s;0];take(s)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_repeat_one_consumes() {
    // Stock rejects this fixture with E0382.
    rejects(
        "repeat_one_consumes",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} fn run()->i32{let s=S{n:7};let a=[s;1];take(s)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_zero_noncopy_whole_move() {
    // Stock rejects this fixture with E0382.
    rejects(
        "zero_noncopy_whole_move",
        r#"struct S{n:i32} fn consume(a:[S;0])->i32{3} fn run()->i32{let a:[S;0]=[];let n=consume(a);n+consume(a)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_immutable_index_assignment() {
    // Stock rejects this fixture with E0594.
    rejects(
        "immutable_index_assignment",
        r#"fn run()->i32{let a=[3,4];a[1]=7;a[1]}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_wrong_array_length() {
    // Stock rejects this fixture with E0308.
    rejects(
        "wrong_array_length",
        r#"fn run()->i32{let a:[i32;2]=[7];a[0]}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_i32_runtime_index() {
    // Stock rejects this fixture with E0277.
    rejects(
        "i32_runtime_index",
        r#"fn index()->i32{1} fn run()->i32{let a=[3,7];a[index()]}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_constant_out_of_bounds() {
    // Stock rejects this out-of-bounds access with unconditional_panic.
    rejects(
        "constant_out_of_bounds",
        r#"fn run()->i32{let a=[3,7];a[2]}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_dynamic_usize_index() {
    runs(
        r#"fn index()->usize{1} fn run()->i32{let a=[3,7];a[index()]}
fn main() { if run() == 7 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_fixed_array_pattern() {
    // Exact stock-valid witness promoted by the bounded Copy-selected array-pattern slice.
    runs(
        r#"fn run()->i32{let [a,b]=[3,7];a*10+b}
fn main() { if run() == 37 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn promotes_unchanged_array_slice_subslice() {
    // Formerly a legal-unsupported boundary; shared subslices are now
    // supported (SUBSLICE.md). Stock rustc 1.98.1 prints exactly this line.
    runs(
        r#"fn run()->i32{let a=[3,7];let s=&a[..];s[1]}
fn main() { if run() == 7 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn accepts_array_len_method() {
    // Stock accepts and prints "array oracle ok"; now native via slice len.
    runs(
        r#"fn run()->i32{let a=[3,7];if a.len()==2{7}else{0}}
fn main() { if run() == 7 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn rejects_invalid_empty_literal_without_context() {
    // Stock rejects this fixture with E0282.
    rejects(
        "empty_literal_without_context",
        r#"fn run()->i32{let a=[];7}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_later_inferred_empty_literal() {
    // Stock accepts and prints "array oracle ok"; now native.
    runs(
        r#"fn take(a:[i32;0])->i32{7} fn run()->i32{let a=[];take(a)}
fn main() { if run() == 7 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}

#[test]
fn rejects_invalid_noncopy_indexed_field_move() {
    // Stock rejects this fixture with E0508.
    rejects(
        "noncopy_indexed_field_move",
        r#"struct S{n:i32} struct R{inner:S} fn take(s:S)->i32{s.n} fn run()->i32{let a=[R{inner:S{n:7}}];take(a[0].inner)}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_assignment_rhs_initializes_root() {
    runs(
        r#"fn run()->i32{let mut a:[i32;1];a[0]={a=[1];7};a[0]}
fn main() { if run() == 7 { println!("array oracle ok"); } }
"#,
        "array oracle ok\n",
    );
}
