//! Native labeled-control CLI regressions, checked against executable stock Rust.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("jr-labels-{}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch: {error}"),
            }
        }
        panic!("could not allocate scratch")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn source(&self, source: &str) -> PathBuf {
        let path = self.path("input.rs");
        fs::write(&path, source).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Output {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    let file = |path: &Path| {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap()
    };
    command
        .stdin(Stdio::null())
        .stdout(file(&stdout))
        .stderr(file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|e| panic!("spawn {command:?}: {e}")),
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
    Output {
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
fn success(result: &Output, context: &str) {
    assert!(
        result.status.success(),
        "{context}: {result:?}\n{}",
        String::from_utf8_lossy(&result.stderr)
    );
}
fn runs(source: &str) {
    let scratch = Scratch::new();
    let input = scratch.source(source);
    let stock = scratch.path("stock");
    // This is an executable oracle, not host constant folding alone. Override for
    // a pinned toolchain without making rustup or network access a prerequisite.
    let rustc = std::env::var_os("JUSTRUST_TEST_RUSTC").unwrap_or_else(|| "rustc".into());
    let mut command = Command::new(rustc);
    command
        .arg(&input)
        .args(["--edition=2024", "-A", "warnings"])
        .arg("-o")
        .arg(&stock);
    success(&capture(&mut command, &scratch, 30), source);
    let oracle = capture(&mut Command::new(&stock), &scratch, 5);
    success(&oracle, source);
    assert_eq!(
        oracle.stdout, b"label oracle ok\n",
        "stock expectation: {source}"
    );
    assert!(oracle.stderr.is_empty());
    for workers in [1, 8] {
        let output = scratch.path(&format!("native-{workers}"));
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            success(
                &result,
                &format!("workers={workers}, check={check}\n{source}"),
            );
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(&mut Command::new(output), &scratch, 5);
        assert_eq!(
            result.status.code(),
            oracle.status.code(),
            "{result:?}\n{source}"
        );
        assert_eq!(result.stdout, oracle.stdout, "workers={workers}\n{source}");
        assert_eq!(result.stderr, oracle.stderr, "workers={workers}\n{source}");
    }
}
fn body(checks: &str) -> String {
    format!(
        "fn run()->bool{{{checks}true}} fn main(){{if run(){{println!(\"label oracle ok\");}}}}"
    )
}
fn rejects(label: &str, source: &str) {
    let scratch = Scratch::new();
    let input = scratch.source(source);
    let rustc = std::env::var_os("JUSTRUST_TEST_RUSTC").unwrap_or_else(|| "rustc".into());
    let mut stock = Command::new(rustc);
    stock
        .arg(&input)
        .args(["--edition=2024", "-A", "warnings"])
        .arg("-o")
        .arg(scratch.path("stock"));
    let oracle = capture(&mut stock, &scratch, 30);
    assert!(!oracle.status.success(), "stock accepted {label}: {source}");
    assert!(oracle.status.code().is_some(), "stock signal: {oracle:?}");
    assert!(
        !oracle.stderr.is_empty(),
        "stock missing diagnostic: {label}"
    );
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
                "{label}: signal: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: no diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: panic: {result:?}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn labeled_loops_while_and_blocks_produce_values() {
    runs(&body(
        r#"
        let a='outer: loop {loop {break 'outer 17;}};
        let b='block: {if true {break 'block 23;} 99};
        let c='fallthrough: {31};
        let unit='unit: {break 'unit;};
        let inner='block: {let mut k=0; loop {k=k+1; if k<2 {continue;} break;} k};
        let mut n=0;
        'count: while n<5 {n=n+1; if n<3 {continue 'count;} break 'count;}
        if a!=17 || b!=23 || c!=31 || inner!=2 || n!=3 {return false;}
    "#,
    ));
}

#[test]
fn nested_break_continue_and_label_shadowing() {
    runs(&body(
        r#"
        let mut n=0; let mut sum=0;
        let result='outer: loop {
            n=n+1;
            'inner: loop {
                if n<3 {continue 'outer;}
                sum=sum+1;
                if sum<2 {continue 'inner;}
                break 'outer n+sum;
            }
        };
        let shadow='same: loop {
            let inner='same: loop {break 'same 7;};
            let block='same: {break 'same 11;};
            break 'same inner+block;
        };
        if result!=5 || n!=3 || sum!=2 || shadow!=18 {return false;}
    "#,
    ));
}

#[test]
fn labeled_exits_restore_pending_operands_and_call_arguments() {
    runs(
        r#"
        fn pair(a:i32,b:i32)->i32{a*10+b}
        fn run()->bool{
            let first='out: loop {100+{loop {break 'out 7;} 0};};
            let second='out: {pair(9,{loop {break 'out 13;}})};
            let mut n=0;
            let third='out: loop {
                n=n+1;
                pair(99,{'block: {if n<3 {continue 'out;} break 'out 19;} 0});
            };
            let mut w=0;
            'count: while w<4 {
                w=w+1;
                pair(55,{loop {if w<3 {continue 'count;} break 'count;}});
            }
            let preserved=10+'block: {break 'block 5;};
            let nested='out: loop {break 'out 'value: {break 'value 29;};};
            if first!=7 || second!=13 || third!=19 || n!=3 || w!=3 || preserved!=15 || nested!=29{return false;}
            true
        }
        fn main(){if run(){println!("label oracle ok");}}
    "#,
    );
}

#[test]
fn invalid_label_targets_and_unlabeled_block_crossings_reject() {
    for (label, source) in [
        ("unknown break", "fn main(){loop {break 'missing;}}"),
        ("unknown continue", "fn main(){loop {continue 'missing;}}"),
        (
            "out of scope label",
            "fn main(){'gone: loop {break;} loop {break 'gone;}}",
        ),
        ("continue block", "fn main(){'block: {continue 'block;}}"),
        (
            "continue shadowed block",
            "fn main(){'same: loop {'same: {continue 'same;}}}",
        ),
        (
            "unlabeled break crosses block",
            "fn main(){loop {'block: {break;}}}",
        ),
        (
            "unlabeled continue crosses block",
            "fn main(){loop {'block: {continue;}}}",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn mismatched_labeled_values_and_while_values_reject() {
    for (label, source) in [
        (
            "loop mixed breaks",
            "fn main(){let x='out: loop {if true {break 'out 1;} break 'out false;};}",
        ),
        (
            "block mixed breaks",
            "fn main(){let x='out: {if true {break 'out 1;} break 'out false;};}",
        ),
        (
            "block tail mismatch",
            "fn main(){let x='out: {if true {break 'out 1;} false};}",
        ),
        (
            "block annotation mismatch",
            "fn main(){let x:bool='out: {break 'out 1;};}",
        ),
        (
            "loop annotation mismatch",
            "fn main(){let x:bool='out: loop {break 'out 1;};}",
        ),
        (
            "unreachable labeled break mismatch",
            "fn main(){let x='out: loop {break 'out 1; break 'out false;};}",
        ),
        (
            "labeled while scalar value",
            "fn main(){'out: while true {break 'out 1;}}",
        ),
        (
            "labeled while unit value",
            "fn main(){'out: while true {break 'out ();}}",
        ),
    ] {
        rejects(label, source);
    }
}
