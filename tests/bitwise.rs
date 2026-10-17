//! Eager bitwise operators, unchanged frozen stock20 corpus.
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
            let path = base.join(format!("jr-bitwise-{}-{epoch}-{id}", std::process::id()));
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
            result.status.code(),
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

#[test]
fn accepts_operations_i8() {
    runs(
        r#"fn bits(a:i8,b:i8)->i8{(a&b)|(a^b)}fn run()->i32{let a:i8=10;let b:i8=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i8 & a)==a{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_i16() {
    runs(
        r#"fn bits(a:i16,b:i16)->i16{(a&b)|(a^b)}fn run()->i32{let a:i16=10;let b:i16=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i16 & a)==a{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_i32() {
    runs(
        r#"fn bits(a:i32,b:i32)->i32{(a&b)|(a^b)}fn run()->i32{let a:i32=10;let b:i32=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i32 & a)==a{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_i64() {
    runs(
        r#"fn bits(a:i64,b:i64)->i64{(a&b)|(a^b)}fn run()->i32{let a:i64=10;let b:i64=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1i64 & a)==a{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_isize() {
    runs(
        r#"fn bits(a:isize,b:isize)->isize{(a&b)|(a^b)}fn run()->i32{let a:isize=10;let b:isize=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6 && (-1isize & a)==a{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_u8() {
    runs(
        r#"fn bits(a:u8,b:u8)->u8{(a&b)|(a^b)}fn run()->i32{let a:u8=10;let b:u8=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_u16() {
    runs(
        r#"fn bits(a:u16,b:u16)->u16{(a&b)|(a^b)}fn run()->i32{let a:u16=10;let b:u16=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_u32() {
    runs(
        r#"fn bits(a:u32,b:u32)->u32{(a&b)|(a^b)}fn run()->i32{let a:u32=10;let b:u32=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_u64() {
    runs(
        r#"fn bits(a:u64,b:u64)->u64{(a&b)|(a^b)}fn run()->i32{let a:u64=10;let b:u64=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_operations_usize() {
    runs(
        r#"fn bits(a:usize,b:usize)->usize{(a&b)|(a^b)}fn run()->i32{let a:usize=10;let b:usize=12;let p=(bits(a,b),[a&b,a|b,a^b]);let r=&p;if r.0==14 && r.1[0]==8 && r.1[1]==14 && r.1[2]==6{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_boolean_eager_truth_tables() {
    runs(
        r#"fn run()->i32{if (true&true) && !(true&false) && (false|true) && !(false|false) && (true^false) && !(true^true){7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_eager_once_left_to_right() {
    runs(
        r#"fn left(p:&mut i32)->bool{*p=*p*10+1;false}fn right(p:&mut i32)->bool{*p=*p*10+2;true}fn run()->i32{let mut n=0;let a=left(&mut n)&right(&mut n);let b=right(&mut n)|left(&mut n);let c=left(&mut n)^right(&mut n);if n==122112 && !a && b && c{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_precedence_and_xor_chain() {
    runs(
        r#"fn run()->i32{let x=1u16|2u16^3u16&1u16;let y=15u8^7u8^3u8;if x==3u16 && y==11u8{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_high64_reference_transport() {
    runs(
        r#"struct S{x:u64,y:i64}fn run()->i32{let s=S{x:18446744073709551615u64,y:-9223372036854775808i64};let r=&s;let p=(r.x&9223372036854775808u64,r.y|1i64);if p.0==9223372036854775808u64 && p.1 == -9223372036854775807i64{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn rejects_invalid_mixed_integer_types() {
    rejects(
        "negative: mixed_integer_types",
        r#"fn run()->i32{let a=1u8;let b=2u16;let x=a&b;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_mixed_boolean_integer() {
    rejects(
        "negative: mixed_boolean_integer",
        r#"fn run()->i32{let x=true|1i32;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_out_of_range_operand() {
    rejects(
        "negative: out_of_range_operand",
        r#"fn run()->i32{let x=256u8^1u8;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_signed_unsigned_same_width() {
    rejects(
        "negative: signed_unsigned_same_width",
        r#"fn run()->i32{let x=1i32^2u32;7}
fn main(){run();}"#,
    );
}

#[test]
fn accepts_compound_assignment() {
    runs(
        r#"fn run()->i32{let mut x=15u8;x&=7u8;x|=8u8;x^=8u8;if x==7u8{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}

#[test]
fn accepts_integer128_bitwise() {
    // Exact source stock/native parity: 20261017-045214157-72203.
    runs(
        r#"fn run()->i32{let x=1u128|2u128;let y=-1i128&7i128;if x==3u128 && y==7i128{7}else{0}}
fn main(){if run()==7{println!("bitwise oracle ok");}}"#,
        "bitwise oracle ok\n",
    );
}
