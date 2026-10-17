//! Phase-one integer foundations, exact frozen stock32 corpus.
//! Declared traps require successful check/build and exact runtime101/stdout.
//! Native panic text/unwinding is not claimed or compared for declared traps.
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
                "jr-integer-types-{}-{epoch}-{id}",
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
fn traps(source: &str, expected: &str) {
    // Only declared runtime traps may differ from stock panic text/unwinding.
    // Exact code 101 rejects signals, crashes, successful exits and all other codes.
    runs_with_exit(source, expected, 101);
}

#[test]
fn accepts_i8_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:i8)->i8{x} fn run()->i32{let lo:i8=-128i8;let hi:i8=127i8;let grouped:i8=-(((128i8)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i8=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_i16_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:i16)->i16{x} fn run()->i32{let lo:i16=-32768i16;let hi:i16=32767i16;let grouped:i16=-(((32768i16)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i16=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_i32_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:i32)->i32{x} fn run()->i32{let lo:i32=-2147483648i32;let hi:i32=2147483647i32;let grouped:i32=-(((2147483648i32)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i32=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_i64_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:i64)->i64{x} fn run()->i32{let lo:i64=-9223372036854775808i64;let hi:i64=9223372036854775807i64;let grouped:i64=-(((9223372036854775808i64)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:i64=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_isize_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:isize)->isize{x} fn run()->i32{let lo:isize=-9223372036854775808isize;let hi:isize=9223372036854775807isize;let grouped:isize=-(((9223372036854775808isize)));let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:isize=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_u8_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:u8)->u8{x} fn run()->i32{let lo:u8=0u8;let hi:u8=255u8;let grouped:u8=0u8;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u8=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_u16_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:u16)->u16{x} fn run()->i32{let lo:u16=0u16;let hi:u16=65535u16;let grouped:u16=0u16;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u16=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_u32_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:u32)->u32{x} fn run()->i32{let lo:u32=0u32;let hi:u32=4294967295u32;let grouped:u32=0u32;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u32=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_u64_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:u64)->u64{x} fn run()->i32{let lo:u64=0u64;let hi:u64=18446744073709551615u64;let grouped:u64=0u64;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:u64=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_usize_bounds_arithmetic_not_transport() {
    runs(
        r#"fn id(x:usize)->usize{x} fn run()->i32{let lo:usize=0usize;let hi:usize=18446744073709551615usize;let grouped:usize=0usize;let a=[id(lo),id(hi)];let p=(a[0],a[1]);let r=&p;let n:usize=3;if r.0==lo && r.1==hi && grouped==lo && !lo==hi && lo<hi && hi>=lo && (n+n)-n==n && n*2==6 && 7/n==2 && 7%n==1{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_radices_and_underscores() {
    runs(
        r#"fn run()->i32{let a:u8=0xff_u8;let b:u16=0b1010_0101u16;let c:i16=0o177_i16;let d:i16=0x7fff_i16;let e:u64=18_446_744_073_709_551_615u64;if a==255u8 && b==165u16 && c==127i16 && d==32767i16 && e==18446744073709551615u64{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_typed_integer_literal_patterns() {
    runs(
        r#"fn small(x:i8)->i32{match x{-128i8=>7,127i8=>3,_=>0}} fn wide(x:u64)->i32{match x{18446744073709551615u64=>7,_=>0}} fn run()->i32{if small(-128i8)==7 && wide(18446744073709551615u64)==7{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_signed_division_remainder_signs() {
    runs(
        r#"fn q_i8(x:i8,y:i8)->i8{x/y} fn r_i8(x:i8,y:i8)->i8{x%y} fn q_i16(x:i16,y:i16)->i16{x/y} fn r_i16(x:i16,y:i16)->i16{x%y} fn q_i32(x:i32,y:i32)->i32{x/y} fn r_i32(x:i32,y:i32)->i32{x%y} fn q_i64(x:i64,y:i64)->i64{x/y} fn r_i64(x:i64,y:i64)->i64{x%y} fn q_isize(x:isize,y:isize)->isize{x/y} fn r_isize(x:isize,y:isize)->isize{x%y} fn run()->i32{if q_i8(-7i8,3i8)==-2i8 && r_i8(-7i8,3i8)==-1i8 && q_i16(-7i16,3i16)==-2i16 && r_i16(-7i16,3i16)==-1i16 && q_i32(-7i32,3i32)==-2i32 && r_i32(-7i32,3i32)==-1i32 && q_i64(-7i64,3i64)==-2i64 && r_i64(-7i64,3i64)==-1i64 && q_isize(-7isize,3isize)==-2isize && r_isize(-7isize,3isize)==-1isize{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_exhaustive_u8_literal_match() {
    runs(
        r#"fn classify(x:u8)->i32{match x{0u8=>7,1u8=>7,2u8=>7,3u8=>7,4u8=>7,5u8=>7,6u8=>7,7u8=>7,8u8=>7,9u8=>7,10u8=>7,11u8=>7,12u8=>7,13u8=>7,14u8=>7,15u8=>7,16u8=>7,17u8=>7,18u8=>7,19u8=>7,20u8=>7,21u8=>7,22u8=>7,23u8=>7,24u8=>7,25u8=>7,26u8=>7,27u8=>7,28u8=>7,29u8=>7,30u8=>7,31u8=>7,32u8=>7,33u8=>7,34u8=>7,35u8=>7,36u8=>7,37u8=>7,38u8=>7,39u8=>7,40u8=>7,41u8=>7,42u8=>7,43u8=>7,44u8=>7,45u8=>7,46u8=>7,47u8=>7,48u8=>7,49u8=>7,50u8=>7,51u8=>7,52u8=>7,53u8=>7,54u8=>7,55u8=>7,56u8=>7,57u8=>7,58u8=>7,59u8=>7,60u8=>7,61u8=>7,62u8=>7,63u8=>7,64u8=>7,65u8=>7,66u8=>7,67u8=>7,68u8=>7,69u8=>7,70u8=>7,71u8=>7,72u8=>7,73u8=>7,74u8=>7,75u8=>7,76u8=>7,77u8=>7,78u8=>7,79u8=>7,80u8=>7,81u8=>7,82u8=>7,83u8=>7,84u8=>7,85u8=>7,86u8=>7,87u8=>7,88u8=>7,89u8=>7,90u8=>7,91u8=>7,92u8=>7,93u8=>7,94u8=>7,95u8=>7,96u8=>7,97u8=>7,98u8=>7,99u8=>7,100u8=>7,101u8=>7,102u8=>7,103u8=>7,104u8=>7,105u8=>7,106u8=>7,107u8=>7,108u8=>7,109u8=>7,110u8=>7,111u8=>7,112u8=>7,113u8=>7,114u8=>7,115u8=>7,116u8=>7,117u8=>7,118u8=>7,119u8=>7,120u8=>7,121u8=>7,122u8=>7,123u8=>7,124u8=>7,125u8=>7,126u8=>7,127u8=>7,128u8=>7,129u8=>7,130u8=>7,131u8=>7,132u8=>7,133u8=>7,134u8=>7,135u8=>7,136u8=>7,137u8=>7,138u8=>7,139u8=>7,140u8=>7,141u8=>7,142u8=>7,143u8=>7,144u8=>7,145u8=>7,146u8=>7,147u8=>7,148u8=>7,149u8=>7,150u8=>7,151u8=>7,152u8=>7,153u8=>7,154u8=>7,155u8=>7,156u8=>7,157u8=>7,158u8=>7,159u8=>7,160u8=>7,161u8=>7,162u8=>7,163u8=>7,164u8=>7,165u8=>7,166u8=>7,167u8=>7,168u8=>7,169u8=>7,170u8=>7,171u8=>7,172u8=>7,173u8=>7,174u8=>7,175u8=>7,176u8=>7,177u8=>7,178u8=>7,179u8=>7,180u8=>7,181u8=>7,182u8=>7,183u8=>7,184u8=>7,185u8=>7,186u8=>7,187u8=>7,188u8=>7,189u8=>7,190u8=>7,191u8=>7,192u8=>7,193u8=>7,194u8=>7,195u8=>7,196u8=>7,197u8=>7,198u8=>7,199u8=>7,200u8=>7,201u8=>7,202u8=>7,203u8=>7,204u8=>7,205u8=>7,206u8=>7,207u8=>7,208u8=>7,209u8=>7,210u8=>7,211u8=>7,212u8=>7,213u8=>7,214u8=>7,215u8=>7,216u8=>7,217u8=>7,218u8=>7,219u8=>7,220u8=>7,221u8=>7,222u8=>7,223u8=>7,224u8=>7,225u8=>7,226u8=>7,227u8=>7,228u8=>7,229u8=>7,230u8=>7,231u8=>7,232u8=>7,233u8=>7,234u8=>7,235u8=>7,236u8=>7,237u8=>7,238u8=>7,239u8=>7,240u8=>7,241u8=>7,242u8=>7,243u8=>7,244u8=>7,245u8=>7,246u8=>7,247u8=>7,248u8=>7,249u8=>7,250u8=>7,251u8=>7,252u8=>7,253u8=>7,254u8=>7,255u8=>7}} fn run()->i32{classify(255u8)}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_mixed_integer_scalar_leaf_transport() {
    runs(
        r#"struct S{a:i8,b:u64,c:i16} enum E{V(i8,u64)} fn relay(x:S)->S{x} fn run()->i32{let s=relay(S{a:-128i8,b:18446744073709551615u64,c:-32768i16});let a=[s.c;2];let r=&s;match E::V(r.a,r.b){E::V(x,y)=>if x==-128i8 && y==18446744073709551615u64 && a[1]==-32768i16{7}else{0}}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn traps_i8_add_overflow() {
    // Stock compiles normally, then exits101: add with overflow.
    traps(
        r#"fn op(x:i8,y:i8)->i8{x+y}
fn main(){println!("before");let _=op(127i8,1i8);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_u8_subtract_overflow() {
    // Stock compiles normally, then exits101: subtract with overflow.
    traps(
        r#"fn op(x:u8,y:u8)->u8{x-y}
fn main(){println!("before");let _=op(0u8,1u8);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_i16_multiply_overflow() {
    // Stock compiles normally, then exits101: multiply with overflow.
    traps(
        r#"fn op(x:i16,y:i16)->i16{x*y}
fn main(){println!("before");let _=op(300i16,300i16);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_i64_add_overflow() {
    // Stock compiles normally, then exits101: add with overflow.
    traps(
        r#"fn op(x:i64,y:i64)->i64{x+y}
fn main(){println!("before");let _=op(9223372036854775807i64,1i64);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_u64_add_overflow() {
    // Stock compiles normally, then exits101: add with overflow.
    traps(
        r#"fn op(x:u64,y:u64)->u64{x+y}
fn main(){println!("before");let _=op(18446744073709551615u64,1u64);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_i32_negate_minimum() {
    // Stock compiles normally, then exits101: negate with overflow.
    traps(
        r#"fn op(x:i32,y:i32)->i32{-x}
fn main(){println!("before");let _=op(-2147483648i32,0i32);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_u16_divide_zero() {
    // Stock compiles normally, then exits101: divide by zero.
    traps(
        r#"fn op(x:u16,y:u16)->u16{x/y}
fn main(){println!("before");let _=op(7u16,0u16);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_i8_minimum_divide_negative_one() {
    // Stock compiles normally, then exits101: divide with overflow.
    traps(
        r#"fn op(x:i8,y:i8)->i8{x/y}
fn main(){println!("before");let _=op(-128i8,-1i8);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn traps_i64_minimum_remainder_negative_one() {
    // Stock compiles normally, then exits101: remainder with overflow.
    traps(
        r#"fn op(x:i64,y:i64)->i64{x%y}
fn main(){println!("before");let _=op(-9223372036854775808i64,-1i64);println!("after");}
"#,
        "before\n",
    );
}

#[test]
fn rejects_invalid_positive_signed_literal_out_of_range() {
    // Stock rejects this fixture with overflowing_literals.
    rejects(
        "positive_signed_literal_out_of_range",
        r#"fn run()->i32{let x:i8=128i8;7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unsigned_literal_out_of_range() {
    // Stock rejects this fixture with overflowing_literals.
    rejects(
        "unsigned_literal_out_of_range",
        r#"fn run()->i32{let x:u8=256u8;7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_signed_literal_too_negative() {
    // Stock rejects this fixture with overflowing_literals.
    rejects(
        "signed_literal_too_negative",
        r#"fn run()->i32{let x:i8=-129i8;7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unsigned_negation_invalid() {
    // Stock rejects this fixture with E0600.
    rejects(
        "unsigned_negation_invalid",
        r#"fn run()->i32{let x:u8=-1u8;7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_equal_width_primitive_identity_mismatch() {
    // Stock rejects this fixture with E0308.
    rejects(
        "equal_width_primitive_identity_mismatch",
        r#"fn run()->i32{let a:i64=1isize;let b:usize=1u64;7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_block_is_not_grouped_minimum() {
    // Stock rejects this fixture with arithmetic_overflow.
    rejects(
        "block_is_not_grouped_minimum",
        r#"fn run()->i32{let x:i8=-{128i8};7}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_integer_128_types() {
    // Promoted after the exact historical source passed stock and native
    // CHECK/BUILD/runtime at both workers 1 and 8.
    runs(
        r#"fn id(x:i128)->i128{x} fn run()->i32{let x=id(-170141183460469231731687303715884105728i128);let y:u128=340282366920938463463374607431768211455u128;if x<0i128 && y>0u128{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}

#[test]
fn accepts_integer_cast_syntax() {
    // Promoted after unchanged-source native build and runtime parity.
    runs(
        r#"fn run()->i32{let x:i8=-1i8;let y=x as u64;if y==18446744073709551615u64{7}else{0}}
fn main() { if run() == 7 { println!("integer types oracle ok"); } }
"#,
        "integer types oracle ok\n",
    );
}
