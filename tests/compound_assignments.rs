//! Builtin compound assignment, unchanged frozen stock32 corpus.
//! Eight declared traps require successful check/build and exact runtime101.
//! Primitive RHS precedes selection and read of the current destination.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/iterator_oracle.rs"]
mod differential;

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
                "jr-compound-assignments-{}-{epoch}-{id}",
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
fn accepts_all_operations_i8() {
    runs(
        r#"struct S{x:i8}fn apply(p:&mut i8){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_i16() {
    runs(
        r#"struct S{x:i16}fn apply(p:&mut i16){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_i32() {
    runs(
        r#"struct S{x:i32}fn apply(p:&mut i32){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_i64() {
    runs(
        r#"struct S{x:i64}fn apply(p:&mut i64){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_isize() {
    runs(
        r#"struct S{x:isize}fn apply(p:&mut isize){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_u8() {
    runs(
        r#"struct S{x:u8}fn apply(p:&mut u8){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_u16() {
    runs(
        r#"struct S{x:u16}fn apply(p:&mut u16){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_u32() {
    runs(
        r#"struct S{x:u32}fn apply(p:&mut u32){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_u64() {
    runs(
        r#"struct S{x:u64}fn apply(p:&mut u64){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_all_operations_usize() {
    runs(
        r#"struct S{x:usize}fn apply(p:&mut usize){*p+=4;*p-=1;*p*=2;*p/=3;*p%=7;*p&=7;*p|=8;*p^=1;*p<<=1u64;*p>>=2i16;}fn run()->i32{let mut s=S{x:12};apply(&mut s.x);let a=[s.x];let p=(a[0],s.x);if p.0==5 && p.1==5{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_rhs_replaces_and_initializes_local() {
    runs(
        r#"fn run()->i32{let mut x=2;x+={x=10;1};let mut y:i32;y+={y=20;2};if x==11 && y==22{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_rhs_retargets_deref() {
    runs(
        r#"fn run()->i32{let mut x=2;let mut y=10;let mut p=&mut x;*p+={p=&mut y;3};if x==2 && y==13{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_dynamic_index_rhs_first_once() {
    runs(
        r#"fn run()->i32{let mut a=[2,10];let mut n=0;a[{n=n*10+1;0usize}]+={n=n*10+2;3};if n==21 && a[0]==5 && a[1]==10{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_rhs_replaces_same_element() {
    runs(
        r#"fn run()->i32{let mut a=[2];a[0]+={a[0]=10;1};if a[0]==11{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_boolean_eager_compounds() {
    runs(
        r#"fn run()->i32{let mut n=0;let mut x=false;x&={n=n+1;true};let mut y=true;y|={n=n+1;false};let mut z=true;z^={n=n+1;true};if n==3 && !x && y && !z{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_heterogeneous_shift_compounds() {
    runs(
        r#"fn run()->i32{let mut x=3u8;x<<=2u64;x>>=1i16;if x==6u8{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn traps_add_overflow() {
    traps(
        r#"fn op(a:i8,y:i8)->i8{let mut x=a;x+=y;x}
fn main(){println!("before");let x=op(127i8,1i8);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_subtract_overflow() {
    traps(
        r#"fn op(a:u8,y:u8)->u8{let mut x=a;x-=y;x}
fn main(){println!("before");let x=op(0u8,1u8);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_multiply_overflow() {
    traps(
        r#"fn op(a:i16,y:i16)->i16{let mut x=a;x*=y;x}
fn main(){println!("before");let x=op(32767i16,2i16);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_divide_zero() {
    traps(
        r#"fn op(a:i32,y:i32)->i32{let mut x=a;x/=y;x}
fn main(){println!("before");let x=op(7i32,0i32);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_remainder_zero() {
    traps(
        r#"fn op(a:u64,y:u64)->u64{let mut x=a;x%=y;x}
fn main(){println!("before");let x=op(7u64,0u64);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_minimum_divide_negative_one() {
    traps(
        r#"fn op(a:i64,y:i64)->i64{let mut x=a;x/=y;x}
fn main(){println!("before");let x=op(-9223372036854775808i64,-1i64);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_negative_shift_count() {
    traps(
        r#"fn op(a:u8,y:i8)->u8{let mut x=a;x<<=y;x}
fn main(){println!("before");let x=op(1u8,-1i8);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn traps_width_shift_count() {
    traps(
        r#"fn op(a:u64,y:u64)->u64{let mut x=a;x>>=y;x}
fn main(){println!("before");let x=op(1u64,64u64);println!("after");}"#,
        "before\n",
    );
}

#[test]
fn rejects_invalid_immutable_target() {
    rejects(
        "negative: immutable_target",
        r#"fn run()->i32{let x=1;x+=2;x}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_uninitialized_target_read() {
    rejects(
        "negative: uninitialized_target_read",
        r#"fn run()->i32{let mut x:i32;x+=1;x}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_shared_reference_write() {
    rejects(
        "negative: shared_reference_write",
        r#"fn run()->i32{let x=1;let p=&x;*p+=1;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_conflicting_live_loan() {
    rejects(
        "negative: conflicting_live_loan",
        r#"fn run()->i32{let mut x=1;let r=&x;x+=1;*r}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_mixed_nonshift_integer_types() {
    rejects(
        "negative: mixed_nonshift_integer_types",
        r#"fn run()->i32{let mut x=1u8;x+=2u16;7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_boolean_arithmetic() {
    rejects(
        "negative: boolean_arithmetic",
        r#"fn run()->i32{let mut x=true;x+=false;7}
fn main(){run();}"#,
    );
}

#[test]
fn accepts_integer128_compounds() {
    // Exact source stock/native parity: 20261017-045214157-72203.
    runs(
        r#"fn run()->i32{let mut x=3u128;x+=4u128;x<<=2u8;x>>=2u8;if x==7u128{7}else{0}}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn accepts_custom_trait_compound_matching_stock() {
    // Formerly a legal-unsupported boundary; user AddAssign compound assignment
    // is now supported by operator traits. Stock rustc 1.98.1 prints this line.
    runs(
        r#"struct S(i32);impl std::ops::AddAssign<i32> for S{fn add_assign(&mut self,r:i32){self.0=self.0+r;}}fn run()->i32{let mut s=S(3);s+=4;s.0}
fn main(){if run()==7{println!("compound oracle ok");}}"#,
        "compound oracle ok\n",
    );
}

#[test]
fn indexed_compound_probe_snapshots_match_stock_distinct_integer_widths() {
    differential::stock_native(
        r#"fn main() {
            let mut narrow=[12u8,7u8]; let i=0usize;
            narrow[i]<<=1u128; narrow[i]>>=1i8;
            let mut wide=[-8i128,16i128]; let j=1usize;
            wide[j]>>=2u8; wide[j]<<=1i16;
            println!("{} {} {} {}",narrow[0],narrow[1],wide[0],wide[1]);
        }"#,
    );
}

#[test]
fn indexed_compound_probe_preserves_rhs_index_order_and_wide_counts() {
    differential::stock_native(
        r#"fn index()->usize { println!("index"); 1usize }
        fn count()->u128 { println!("rhs"); 64u128 }
        fn update(values:&mut [u128;2]) {
            let mut selected=0usize;
            values[selected] <<= { selected=1usize; println!("move rhs"); 64u128 };
            values[index()] >>= count();
        }
        fn main() {
            let mut values=[1u128,2u128]; update(&mut values);
            let mut signed=[-8i128,-73786976294838206464i128];
            let selected=1usize; signed[selected] >>= 65u128;
            let mut narrow=[-8i32,-8i32];
            narrow[selected] >>= 1i128; narrow[selected] <<= 1u128;
            let mut flags=[true,false]; let mut trace=0;
            flags[{trace=trace*10+1; 0usize}] &= {trace=trace*10+2; false};
            println!("{} {} {} {} {} {}", values[0],values[1],signed[1],narrow[1],flags[0],trace);
        }"#,
    );
}
