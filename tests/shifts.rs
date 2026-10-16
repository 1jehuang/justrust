//! Checked heterogeneous shifts, unchanged frozen stock32 corpus.
//! Native CHECK rejects static invalid counts earlier than stock metadata-only
//! checking. Stock BUILD rejects those same sources under arithmetic_overflow.
//! Declared runtime traps require successful check/build and exact exit101.
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
            let path = base.join(format!("jr-shifts-{}-{epoch}-{id}", std::process::id()));
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
fn traps(source: &str, expected: &str) {
    // Only declared runtime traps may differ from stock panic text/unwinding.
    // Exact code 101 rejects signals, crashes, successful exits and all other codes.
    runs_with_exit(source, expected, 101);
}

#[test]
fn accepts_shift_i8_edges() {
    runs(
        r#"fn left(x:i8,n:u64)->i8{x<<n}fn right(x:i8,n:i16)->i8{x>>n}fn run()->i32{let a=left(1i8,7u64);let b=left(-1i8,1u64);let z=right(-128i8,7i16);if a==-128i8 && b==-2i8 && z==-1i8{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_i16_edges() {
    runs(
        r#"fn left(x:i16,n:u64)->i16{x<<n}fn right(x:i16,n:i16)->i16{x>>n}fn run()->i32{let a=left(1i16,15u64);let b=left(-1i16,1u64);let z=right(-32768i16,15i16);if a==-32768i16 && b==-2i16 && z==-1i16{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_i32_edges() {
    runs(
        r#"fn left(x:i32,n:u64)->i32{x<<n}fn right(x:i32,n:i16)->i32{x>>n}fn run()->i32{let a=left(1i32,31u64);let b=left(-1i32,1u64);let z=right(-2147483648i32,31i16);if a==-2147483648i32 && b==-2i32 && z==-1i32{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_i64_edges() {
    runs(
        r#"fn left(x:i64,n:u64)->i64{x<<n}fn right(x:i64,n:i16)->i64{x>>n}fn run()->i32{let a=left(1i64,63u64);let b=left(-1i64,1u64);let z=right(-9223372036854775808i64,63i16);if a==-9223372036854775808i64 && b==-2i64 && z==-1i64{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_isize_edges() {
    runs(
        r#"fn left(x:isize,n:u64)->isize{x<<n}fn right(x:isize,n:i16)->isize{x>>n}fn run()->i32{let a=left(1isize,63u64);let b=left(-1isize,1u64);let z=right(-9223372036854775808isize,63i16);if a==-9223372036854775808isize && b==-2isize && z==-1isize{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_u8_edges() {
    runs(
        r#"fn left(x:u8,n:u64)->u8{x<<n}fn right(x:u8,n:i16)->u8{x>>n}fn run()->i32{let a=left(1u8,7u64);let b=left(255u8,1u64);let z=right(128u8,7i16);if a==128u8 && b==254u8 && z==1u8{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_u16_edges() {
    runs(
        r#"fn left(x:u16,n:u64)->u16{x<<n}fn right(x:u16,n:i16)->u16{x>>n}fn run()->i32{let a=left(1u16,15u64);let b=left(65535u16,1u64);let z=right(32768u16,15i16);if a==32768u16 && b==65534u16 && z==1u16{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_u32_edges() {
    runs(
        r#"fn left(x:u32,n:u64)->u32{x<<n}fn right(x:u32,n:i16)->u32{x>>n}fn run()->i32{let a=left(1u32,31u64);let b=left(4294967295u32,1u64);let z=right(2147483648u32,31i16);if a==2147483648u32 && b==4294967294u32 && z==1u32{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_u64_edges() {
    runs(
        r#"fn left(x:u64,n:u64)->u64{x<<n}fn right(x:u64,n:i16)->u64{x>>n}fn run()->i32{let a=left(1u64,63u64);let b=left(18446744073709551615u64,1u64);let z=right(9223372036854775808u64,63i16);if a==9223372036854775808u64 && b==18446744073709551614u64 && z==1u64{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_shift_usize_edges() {
    runs(
        r#"fn left(x:usize,n:u64)->usize{x<<n}fn right(x:usize,n:i16)->usize{x>>n}fn run()->i32{let a=left(1usize,63u64);let b=left(18446744073709551615usize,1u64);let z=right(9223372036854775808usize,63i16);if a==9223372036854775808usize && b==18446744073709551614usize && z==1usize{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_once_left_to_right() {
    runs(
        r#"fn lhs(p:&mut i32)->u16{*p=*p*10+1;3u16}fn rhs(p:&mut i32)->u8{*p=*p*10+2;2u8}fn run()->i32{let mut n=0;let x=lhs(&mut n)<<rhs(&mut n);if n==12 && x==12u16{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_precedence_cast_arithmetic_bitwise() {
    runs(
        r#"fn run()->i32{let x=1u16<<1u8+2u8;let y=1u16|2u16<<2u8;let z=256u16 as u8>>1u8;if x==8u16 && y==9u16 && z==0u8{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_left_associative_chain() {
    runs(
        r#"fn run()->i32{let x=1u16<<3u8>>1i64;if x==4u16{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_left_context_rhs_independent() {
    runs(
        r#"fn run()->i32{let x:u8=1<<2u64;let y:i16=-8>>1u8;if x==4u8 && y == -4i16{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn accepts_reference_aggregate_transport() {
    runs(
        r#"struct S{x:u64,y:i64}fn run()->i32{let s=S{x:9223372036854775808u64,y:-8i64};let r=&s;let a=[r.x>>63u8];let p=(a[0],r.y>>2usize);if p.0==1u64 && p.1 == -2i64{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn traps_negative_left() {
    traps(
        r#"fn op(x:u8,n:i8)->u8{x<<n}
fn main(){println!("before");let x=op(1u8,-1i8);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_negative_right() {
    traps(
        r#"fn op(x:i64,n:i64)->i64{x>>n}
fn main(){println!("before");let x=op(1i64,-1i64);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_exact_width_u8() {
    traps(
        r#"fn op(x:u8,n:u16)->u8{x<<n}
fn main(){println!("before");let x=op(1u8,8u16);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_exact_width_i64() {
    traps(
        r#"fn op(x:i64,n:u8)->i64{x>>n}
fn main(){println!("before");let x=op(1i64,64u8);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_exact_width_usize() {
    traps(
        r#"fn op(x:usize,n:usize)->usize{x<<n}
fn main(){println!("before");let x=op(1usize,64usize);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_huge_u64_left() {
    traps(
        r#"fn op(x:u8,n:u64)->u8{x<<n}
fn main(){println!("before");let x=op(1u8,18446744073709551615u64);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_huge_u64_right() {
    traps(
        r#"fn op(x:u64,n:u64)->u64{x>>n}
fn main(){println!("before");let x=op(1u64,18446744073709551615u64);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_negative_minimum() {
    traps(
        r#"fn op(x:u16,n:i8)->u16{x>>n}
fn main(){println!("before");let x=op(1u16,-128i8);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_above_width_u32() {
    traps(
        r#"fn op(x:u32,n:u64)->u32{x<<n}
fn main(){println!("before");let x=op(1u32,4294967296u64);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn rejects_invalid_bool_left() {
    rejects(
        "negative: bool_left",
        r#"fn run()->i32{let x=true<<1u8;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_bool_count() {
    rejects(
        "negative: bool_count",
        r#"fn run()->i32{let x=1u8>>true;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_constant_width_overflow() {
    // Stock metadata accepts, but BUILD rejects. Native CHECK rejects earlier.
    rejects(
        "negative: constant_width_overflow",
        r#"fn run()->i32{let x=1u8<<8u8;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_constant_negative_count() {
    // Stock metadata accepts, but BUILD rejects. Native CHECK rejects earlier.
    rejects(
        "negative: constant_negative_count",
        r#"fn run()->i32{let x=1u16>>-1i8;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_source_literal_range() {
    rejects(
        "negative: source_literal_range",
        r#"fn run()->i32{let x=256u8<<0u8;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_count_literal_range() {
    rejects(
        "negative: count_literal_range",
        r#"fn run()->i32{let x=1u8<<256u8;7}
fn main(){run();}"#,
    );
}

#[test]
fn accepts_compound_shift_assignment() {
    runs(
        r#"fn run()->i32{let mut x=1u8;x<<=3u16;x>>=1i8;if x==4u8{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
        "shift oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_integer128_shifts() {
    rejects(
        "legalunsupported: integer128_shifts",
        r#"fn run()->i32{let x=1u128<<100u8;let y=x>>100u8;if y==1u128{7}else{0}}
fn main(){if run()==7{println!("shift oracle ok");}}"#,
    );
}
