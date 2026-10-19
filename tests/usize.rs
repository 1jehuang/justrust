//! Full-width usize and dynamic indexing bounded native acceptance.
//! All 32 exact fixtures were stock-classified with rustc 1.98.1, edition 2024.
//! Stock is an external scratch oracle only and is never invoked by these tests.
//! Arithmetic requires SIGABRT; legacy bounds guards retain exit 101/stdout.
//! Native panic text/unwinding is not claimed or compared for those trap fixtures.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
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
            let path = base.join(format!("jr-usize-{}-{epoch}-{id}", std::process::id()));
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
fn runs_with_exit(source: &str, expected: &str, expected_exit: i32) {
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
        assert_eq!(
            if expected_exit < 0 {
                result.status.signal().map(|signal| -signal)
            } else {
                result.status.code()
            },
            Some(expected_exit),
            "runtime status failure: {result:?}\n{source}"
        );
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        if expected_exit == 0 {
            assert!(
                result.stderr.is_empty(),
                "unexpected runtime stderr: {result:?}"
            );
        }
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

fn runs(source: &str, expected: &str) {
    runs_with_exit(source, expected, 0);
}
fn arithmetic_traps(source: &str, expected: &str) {
    runs_with_exit(source, expected, -6);
}
fn traps(source: &str, expected: &str) {
    // Only declared runtime traps may differ from stock panic text/unwinding.
    // Exact code 101 rejects signals, crashes, successful exits and all other codes.
    runs_with_exit(source, expected, 101);
}

#[test]
fn accepts_full64_function_transport() {
    runs(
        r#"fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn identity(x:usize)->usize{x} fn run()->i32{if identity(wide())==4294967296{if identity(high())==9223372036854775808{if identity(maximum())==18446744073709551615{7i32}else{0}}else{0}}else{0}}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_aggregate_usize_transport() {
    runs(
        r#"fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} struct S{n:usize} enum E{A(usize)} fn run()->i32{let t=(wide(),maximum());let s=S{n:t.0};let a=[t.1;2];match E::A(s.n){E::A(n)=>if n==wide(){if a[1]==maximum(){7}else{0}}else{0}}}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_unsigned_comparisons() {
    runs(
        r#"fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn run()->i32{let z:usize=0;if high()>wide() && maximum()>high() && z<maximum() && maximum()>=high() && high()<=maximum() && high()!=wide() && z==z{7}else{0}}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_wide_checked_arithmetic() {
    runs(
        r#"fn run()->i32{let a:usize=4294967296;let b:usize=7;let x=(a+b)*b-b;let h:usize=9223372036854775807;let one:usize=1;let m:usize=4611686018427387904;let two:usize=2;if x==30064771114{if h+one==9223372036854775808{if m*two==9223372036854775808{7}else{0}}else{0}}else{0}}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_unsigned_division_remainder() {
    runs(
        r#"fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn run()->i32{let d:usize=3;if high()/d==3074457345618258602{if high()%d==2{7}else{0}}else{0}}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_dynamic_scalar_read() {
    runs(
        r#"fn index()->usize{1} fn run()->i32{let a=[3,7];a[index()]}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_copy_tuple_dynamic_read() {
    runs(
        r#"fn index()->usize{1} fn run()->i32{let a=[(1,2),(3,7)];let p=a[index()];p.0*10+p.1}
fn main() { if run() == 37 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_copy_array_dynamic_read() {
    runs(
        r#"fn index()->usize{1} fn run()->i32{let a=[[1,2],[3,7]];let p=a[index()];p[0]*10+p[1]}
fn main() { if run() == 37 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_read_base_then_index() {
    runs(
        r#"fn base()->[i32;2]{println!("base");[3,7]} fn index()->usize{println!("index");1} fn run()->i32{base()[index()]}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "base\nindex\ndynamic oracle ok\n",
    );
}

#[test]
fn accepts_assignment_rhs_then_index() {
    runs(
        r#"fn index()->usize{println!("index");1} fn rhs()->i32{println!("rhs");7} fn run()->i32{let mut a=[3,4];a[index()]=rhs();a[1]}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "rhs\nindex\ndynamic oracle ok\n",
    );
}

#[test]
fn accepts_index_replaces_live_array() {
    runs(
        r#"fn run()->i32{let mut a=[3];a[{a=[7];0}]}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_wildcard_and_replace() {
    runs(
        r#"struct S{n:i32} fn index()->usize{0} fn run()->i32{let mut a=[S{n:3}];let _=a[index()];a[index()]=S{n:7};a[0].n}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn accepts_assignment_rhs_initializes_array() {
    runs(
        r#"fn index()->usize{0} fn run()->i32{let mut a:[i32;1];a[index()]={a=[3];7};a[0]}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn rejects_invalid_i32_dynamic_index() {
    // Stock rejects this fixture with E0277.
    rejects(
        "i32_dynamic_index",
        r#"fn index()->i32{1} fn run()->i32{let a=[3,7];a[index()]}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_noncopy_dynamic_move() {
    // Stock rejects this fixture with E0508.
    rejects(
        "noncopy_dynamic_move",
        r#"struct S{n:i32} fn index()->usize{0} fn take(s:S)->i32{s.n} fn run()->i32{let a=[S{n:7}];take(a[index()])}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_usize_unary_negation() {
    // Stock rejects this fixture with E0600.
    rejects(
        "usize_unary_negation",
        r#"fn run()->i32{let x:usize=7;let y=-x;0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_usize_literal_overflow() {
    // Stock rejects this out-of-range usize literal.
    rejects(
        "usize_literal_overflow",
        r#"fn run()->i32{let x:usize=18446744073709551616;0}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_index_initializes_uninitialized_array() {
    runs(
        r#"fn run()->i32{let a:[i32;1];let x=a[{a=[7];0}];x}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn traps_checked_add_overflow() {
    // Stock runtime panic verified: attempt to add with overflow.
    arithmetic_traps(
        r#"fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn add(a:usize,b:usize)->usize{a+b} fn run()->i32{println!("before");let one:usize=1;let n=add(maximum(),one);0}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_checked_sub_underflow() {
    // Stock runtime panic verified: attempt to subtract with overflow.
    arithmetic_traps(
        r#"fn sub(a:usize,b:usize)->usize{a-b} fn run()->i32{println!("before");let zero:usize=0;let one:usize=1;let n=sub(zero,one);0}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_checked_mul_overflow() {
    // Stock runtime panic verified: attempt to multiply with overflow.
    arithmetic_traps(
        r#"fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn mul(a:usize,b:usize)->usize{a*b} fn run()->i32{println!("before");let two:usize=2;let n=mul(high(),two);0}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_usize_divide_by_zero() {
    // Stock runtime panic verified: attempt to divide by zero.
    arithmetic_traps(
        r#"fn divide(a:usize,b:usize)->usize{a/b} fn run()->i32{println!("before");let a:usize=7;let b:usize=0;let n=divide(a,b);0}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_usize_remainder_by_zero() {
    // Stock runtime panic verified: attempt to calculate the remainder with a divisor of zero.
    arithmetic_traps(
        r#"fn rem(a:usize,b:usize)->usize{a%b} fn run()->i32{println!("before");let a:usize=7;let b:usize=0;let n=rem(a,b);0}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_bounds_equal_length() {
    // Stock runtime panic verified: index out of bounds.
    traps(
        r#"fn index()->usize{2} fn run()->i32{println!("before");let a=[3,7];a[index()]}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_bounds_no_low32_truncation() {
    // Stock runtime panic verified: index out of bounds.
    traps(
        r#"fn index()->usize{4294967296} fn run()->i32{println!("before");let a=[3,7];a[index()]}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_bounds_highbit_unsigned() {
    // Stock runtime panic verified: index out of bounds.
    traps(
        r#"fn index()->usize{9223372036854775808} fn run()->i32{println!("before");let a=[3,7];a[index()]}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn traps_bounds_empty_array() {
    // Stock runtime panic verified: index out of bounds.
    traps(
        r#"fn index()->usize{0} fn run()->i32{println!("before");let a:[i32;0]=[];a[index()]}
fn main() { run(); println!("after trap"); }
"#,
        "before\n",
    );
}

#[test]
fn rejects_legal_unsupported_backward_index_inference() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "backward_index_inference",
        r#"fn run()->i32{let i=1;let a=[3,7];a[i]}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_noncopy_dynamic_copy_field() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "noncopy_dynamic_copy_field",
        r#"struct S{n:i32} fn index()->usize{0} fn run()->i32{let a=[S{n:7}];a[index()].n}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
    );
}

#[test]
fn accepts_usize_stack_argument_transport() {
    runs(
        r#"fn wide()->usize{4294967296usize} fn high()->usize{9223372036854775808usize} fn maximum()->usize{18446744073709551615usize} fn pick(a:usize,b:usize,c:usize,d:usize,e:usize,f:usize,g:usize,h:usize)->usize{h} fn run()->i32{let z:usize=0;if pick(z,z,z,z,z,z,z,maximum())==maximum(){7}else{0}}
fn main() { if run() == 7 { println!("dynamic oracle ok"); } }
"#,
        "dynamic oracle ok\n",
    );
}

#[test]
fn rejects_invalid_static_prefix_requires_readiness() {
    // Stock rejects this fixture with E0381.
    rejects(
        "static_prefix_requires_readiness",
        r#"fn run()->i32{let a:[[i32;1];1];a[0][{a=[[7]];0}]}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_mixed_concrete_arithmetic() {
    // Stock rejects this fixture with E0308.
    rejects(
        "mixed_concrete_arithmetic",
        r#"fn run()->i32{let a:usize=7;let b:i32=3;let n=a+b;0}
fn main() { run(); }
"#,
    );
}
