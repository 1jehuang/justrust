//! Integer casts, unchanged frozen stock32 corpus.
//! All admitted conversions run successfully, including narrowing.
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
                "jr-integer-casts-{}-{epoch}-{id}",
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
fn accepts_matrix_from_i8() {
    runs(
        r#"fn check0(x:i8)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:i8)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:i8)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}
fn check3(x:i8)->i32{if (x as i8) != -128i8 {return 0;}if (x as i16) != -128i16 {return 0;}if (x as i32) != -128i32 {return 0;}if (x as i64) != -128i64 {return 0;}if (x as isize) != -128isize {return 0;}if (x as u8) != 128u8 {return 0;}if (x as u16) != 65408u16 {return 0;}if (x as u32) != 4294967168u32 {return 0;}if (x as u64) != 18446744073709551488u64 {return 0;}if (x as usize) != 18446744073709551488usize {return 0;}7}
fn check4(x:i8)->i32{if (x as i8) != 127i8 {return 0;}if (x as i16) != 127i16 {return 0;}if (x as i32) != 127i32 {return 0;}if (x as i64) != 127i64 {return 0;}if (x as isize) != 127isize {return 0;}if (x as u8) != 127u8 {return 0;}if (x as u16) != 127u16 {return 0;}if (x as u32) != 127u32 {return 0;}if (x as u64) != 127u64 {return 0;}if (x as usize) != 127usize {return 0;}7}
fn run()->i32{if check0(0i8)!=7{return 0;}if check1(1i8)!=7{return 0;}if check2(-1i8)!=7{return 0;}if check3(-128i8)!=7{return 0;}if check4(127i8)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_i16() {
    runs(
        r#"fn check0(x:i16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:i16)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:i16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}
fn check3(x:i16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != -32768i16 {return 0;}if (x as i32) != -32768i32 {return 0;}if (x as i64) != -32768i64 {return 0;}if (x as isize) != -32768isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 32768u16 {return 0;}if (x as u32) != 4294934528u32 {return 0;}if (x as u64) != 18446744073709518848u64 {return 0;}if (x as usize) != 18446744073709518848usize {return 0;}7}
fn check4(x:i16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != 32767i16 {return 0;}if (x as i32) != 32767i32 {return 0;}if (x as i64) != 32767i64 {return 0;}if (x as isize) != 32767isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 32767u16 {return 0;}if (x as u32) != 32767u32 {return 0;}if (x as u64) != 32767u64 {return 0;}if (x as usize) != 32767usize {return 0;}7}
fn run()->i32{if check0(0i16)!=7{return 0;}if check1(1i16)!=7{return 0;}if check2(-1i16)!=7{return 0;}if check3(-32768i16)!=7{return 0;}if check4(32767i16)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_i32() {
    runs(
        r#"fn check0(x:i32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:i32)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:i32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}
fn check3(x:i32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != -2147483648i32 {return 0;}if (x as i64) != -2147483648i64 {return 0;}if (x as isize) != -2147483648isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 2147483648u32 {return 0;}if (x as u64) != 18446744071562067968u64 {return 0;}if (x as usize) != 18446744071562067968usize {return 0;}7}
fn check4(x:i32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != 2147483647i32 {return 0;}if (x as i64) != 2147483647i64 {return 0;}if (x as isize) != 2147483647isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 2147483647u32 {return 0;}if (x as u64) != 2147483647u64 {return 0;}if (x as usize) != 2147483647usize {return 0;}7}
fn run()->i32{if check0(0i32)!=7{return 0;}if check1(1i32)!=7{return 0;}if check2(-1i32)!=7{return 0;}if check3(-2147483648i32)!=7{return 0;}if check4(2147483647i32)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_i64() {
    runs(
        r#"fn check0(x:i64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:i64)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:i64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}
fn check3(x:i64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}
fn check4(x:i64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 9223372036854775807i64 {return 0;}if (x as isize) != 9223372036854775807isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 9223372036854775807u64 {return 0;}if (x as usize) != 9223372036854775807usize {return 0;}7}
fn run()->i32{if check0(0i64)!=7{return 0;}if check1(1i64)!=7{return 0;}if check2(-1i64)!=7{return 0;}if check3(-9223372036854775808i64)!=7{return 0;}if check4(9223372036854775807i64)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_isize() {
    runs(
        r#"fn check0(x:isize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:isize)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:isize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}
fn check3(x:isize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}
fn check4(x:isize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 9223372036854775807i64 {return 0;}if (x as isize) != 9223372036854775807isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 9223372036854775807u64 {return 0;}if (x as usize) != 9223372036854775807usize {return 0;}7}
fn run()->i32{if check0(0isize)!=7{return 0;}if check1(1isize)!=7{return 0;}if check2(-1isize)!=7{return 0;}if check3(-9223372036854775808isize)!=7{return 0;}if check4(9223372036854775807isize)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_u8() {
    runs(
        r#"fn check0(x:u8)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:u8)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:u8)->i32{if (x as i8) != -128i8 {return 0;}if (x as i16) != 128i16 {return 0;}if (x as i32) != 128i32 {return 0;}if (x as i64) != 128i64 {return 0;}if (x as isize) != 128isize {return 0;}if (x as u8) != 128u8 {return 0;}if (x as u16) != 128u16 {return 0;}if (x as u32) != 128u32 {return 0;}if (x as u64) != 128u64 {return 0;}if (x as usize) != 128usize {return 0;}7}
fn check3(x:u8)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != 255i16 {return 0;}if (x as i32) != 255i32 {return 0;}if (x as i64) != 255i64 {return 0;}if (x as isize) != 255isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 255u16 {return 0;}if (x as u32) != 255u32 {return 0;}if (x as u64) != 255u64 {return 0;}if (x as usize) != 255usize {return 0;}7}
fn run()->i32{if check0(0u8)!=7{return 0;}if check1(1u8)!=7{return 0;}if check2(128u8)!=7{return 0;}if check3(255u8)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_u16() {
    runs(
        r#"fn check0(x:u16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:u16)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:u16)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != -32768i16 {return 0;}if (x as i32) != 32768i32 {return 0;}if (x as i64) != 32768i64 {return 0;}if (x as isize) != 32768isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 32768u16 {return 0;}if (x as u32) != 32768u32 {return 0;}if (x as u64) != 32768u64 {return 0;}if (x as usize) != 32768usize {return 0;}7}
fn check3(x:u16)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != 65535i32 {return 0;}if (x as i64) != 65535i64 {return 0;}if (x as isize) != 65535isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 65535u32 {return 0;}if (x as u64) != 65535u64 {return 0;}if (x as usize) != 65535usize {return 0;}7}
fn run()->i32{if check0(0u16)!=7{return 0;}if check1(1u16)!=7{return 0;}if check2(32768u16)!=7{return 0;}if check3(65535u16)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_u32() {
    runs(
        r#"fn check0(x:u32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:u32)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:u32)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != -2147483648i32 {return 0;}if (x as i64) != 2147483648i64 {return 0;}if (x as isize) != 2147483648isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 2147483648u32 {return 0;}if (x as u64) != 2147483648u64 {return 0;}if (x as usize) != 2147483648usize {return 0;}7}
fn check3(x:u32)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != 4294967295i64 {return 0;}if (x as isize) != 4294967295isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 4294967295u64 {return 0;}if (x as usize) != 4294967295usize {return 0;}7}
fn run()->i32{if check0(0u32)!=7{return 0;}if check1(1u32)!=7{return 0;}if check2(2147483648u32)!=7{return 0;}if check3(4294967295u32)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_u64() {
    runs(
        r#"fn check0(x:u64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:u64)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:u64)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}
fn check3(x:u64)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}
fn run()->i32{if check0(0u64)!=7{return 0;}if check1(1u64)!=7{return 0;}if check2(9223372036854775808u64)!=7{return 0;}if check3(18446744073709551615u64)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_matrix_from_usize() {
    runs(
        r#"fn check0(x:usize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != 0i64 {return 0;}if (x as isize) != 0isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 0u64 {return 0;}if (x as usize) != 0usize {return 0;}7}
fn check1(x:usize)->i32{if (x as i8) != 1i8 {return 0;}if (x as i16) != 1i16 {return 0;}if (x as i32) != 1i32 {return 0;}if (x as i64) != 1i64 {return 0;}if (x as isize) != 1isize {return 0;}if (x as u8) != 1u8 {return 0;}if (x as u16) != 1u16 {return 0;}if (x as u32) != 1u32 {return 0;}if (x as u64) != 1u64 {return 0;}if (x as usize) != 1usize {return 0;}7}
fn check2(x:usize)->i32{if (x as i8) != 0i8 {return 0;}if (x as i16) != 0i16 {return 0;}if (x as i32) != 0i32 {return 0;}if (x as i64) != -9223372036854775808i64 {return 0;}if (x as isize) != -9223372036854775808isize {return 0;}if (x as u8) != 0u8 {return 0;}if (x as u16) != 0u16 {return 0;}if (x as u32) != 0u32 {return 0;}if (x as u64) != 9223372036854775808u64 {return 0;}if (x as usize) != 9223372036854775808usize {return 0;}7}
fn check3(x:usize)->i32{if (x as i8) != -1i8 {return 0;}if (x as i16) != -1i16 {return 0;}if (x as i32) != -1i32 {return 0;}if (x as i64) != -1i64 {return 0;}if (x as isize) != -1isize {return 0;}if (x as u8) != 255u8 {return 0;}if (x as u16) != 65535u16 {return 0;}if (x as u32) != 4294967295u32 {return 0;}if (x as u64) != 18446744073709551615u64 {return 0;}if (x as usize) != 18446744073709551615usize {return 0;}7}
fn run()->i32{if check0(0usize)!=7{return 0;}if check1(1usize)!=7{return 0;}if check2(9223372036854775808usize)!=7{return 0;}if check3(18446744073709551615usize)!=7{return 0;}7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_suffixed_narrow() {
    runs(
        r#"fn run()->i32{let x=300i32 as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_binary_drops_hint() {
    runs(
        r#"fn run()->i32{let x=(200+100) as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_block_tail_hint() {
    runs(
        r#"fn run()->i32{let x={let y=1;127+y-1;255} as u8;if x==255u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_block_binary_tail() {
    runs(
        r#"fn run()->i32{let x={200+100} as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_if_drops_hint() {
    runs(
        r#"fn run()->i32{let x=(if true{300}else{1}) as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_match_drops_hint() {
    runs(
        r#"fn run()->i32{let x=(match true{true=>300,false=>1}) as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_loop_drops_hint() {
    runs(
        r#"fn run()->i32{let x=(loop{break 300;}) as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_not_forwards_hint() {
    runs(
        r#"fn run()->i32{let x=(!0) as u8;if x==255u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_negative_signed_hint() {
    runs(
        r#"fn run()->i32{let x=(-128) as i8;if x == -128i8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_nested_cast_own_hint() {
    runs(
        r#"fn run()->i32{let x=(255 as u8) as i16;if x==255i16{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_left_associative_cast_chain() {
    runs(
        r#"fn run()->i32{let x=511i32 as u8 as i16;if x==255i16{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_unary_cast_multiply_precedence() {
    runs(
        r#"fn run()->i32{let x=-2i16 as u8 * 1u8;if x==254u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_concrete_local_field_index() {
    runs(
        r#"struct S{x:i16}fn run()->i32{let s=S{x:-1i16};let a=[300i32];let v=65535u16;if s.x as u32==4294967295u32 && a[0] as u8==44u8 && v as i16 == -1i16{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_reference_projection_last_use() {
    runs(
        r#"struct S{x:i16}fn run()->i32{let mut s=S{x:-1i16};let r=&s;let n=r.x as u64;s.x=7i16;if n==18446744073709551615u64 && s.x==7i16{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_once_evaluation() {
    runs(
        r#"fn run()->i32{let mut n=0;let x={n=n+1;300i32} as u8;if n==1 && x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn accepts_concrete_call() {
    runs(
        r#"fn value()->i32{300}fn run()->i32{let x=value() as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
        "cast oracle ok\n",
    );
}

#[test]
fn rejects_invalid_unsuffixed_source_out_of_range() {
    rejects(
        "negative: unsuffixed_source_out_of_range",
        r#"fn run()->i32{let x=300 as u8;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_unsigned_negative_hint() {
    rejects(
        "negative: unsigned_negative_hint",
        r#"fn run()->i32{let x=(-1) as u8;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_suffixed_source_out_of_range() {
    rejects(
        "negative: suffixed_source_out_of_range",
        r#"fn run()->i32{let x=256u8 as u16;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_legal_unsupported_bool_to_integer() {
    rejects(
        "legalunsupported: bool_to_integer",
        r#"fn run()->i32{if true as u8==1u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
    );
}

#[test]
fn rejects_legal_unsupported_reference_to_pointer() {
    rejects(
        "legalunsupported: reference_to_pointer",
        r#"fn run()->i32{let x=7;let p=&x as *const i32;7}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
    );
}

#[test]
fn rejects_legal_unsupported_integer128_cast() {
    rejects(
        "legalunsupported: integer128_cast",
        r#"fn run()->i32{let x=300u128 as u8;if x==44u8{7}else{0}}
fn main(){if run()==7{println!("cast oracle ok");}}"#,
    );
}
