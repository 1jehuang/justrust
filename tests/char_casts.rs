//! Bounded char casts, independently stock-measured before native expectations.
//! Stock: /usr/bin/rustc, edition 2024; 16 executable positives and 22 type rejections.
//! Exact rustc -vV, source hashes, command stdout/stderr/status and runtime receipts:
//! /home/jeremy/.jcode/scratch/char-casts-stock-20261018/results.json
//! Receipt SHA256: 8f08e56b4ba29908641f980bc5894c1b490be79a18e4d723b0daee42e3b024ab
//! All native rejection expectations are type errors, never unsupported feature passes.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::{
    Key,
    compiler::{self, frontend},
};
use std::fs::{self, OpenOptions};
use std::os::unix::fs::PermissionsExt;
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
            let path = base.join(format!("jr-char-casts-{}-{id}", std::process::id()));
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Boundary {
    Live,
    Syntax,
    ParseUnsupported,
    Type,
}
struct Case {
    name: &'static str,
    source: &'static str,
    bytes: usize,
    sha256: &'static str,
    boundary: Boundary,
    stock_compile_code: i32,
    stock_compile_stderr: &'static str,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: r###"char_to_u8"###,
            source: r###"fn run()->bool{let c='\u{100080}';let n:u8=c as u8;n==128u8}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 114,
            sha256: r###"0da3d8d4c6c5351688915c2b4fa6ba918f44228363ad74ff4460162bf434d577"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_u16"###,
            source: r###"fn run()->bool{let c='\u{ffff}';let n:u16=c as u16;n==65535u16}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 117,
            sha256: r###"a0456b369e6168888d49b0414587d64263dc45416019eeed2f1d13da66ac17c6"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_u32"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:u32=c as u32;n==1114111u32}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 121,
            sha256: r###"97b14575e3fb0590f56ac12abbf992b777e902f4f5e85d18daf480b083e08ef5"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_u64"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:u64=c as u64;n==1114111u64}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 121,
            sha256: r###"0da24c72aa90e02984267378b23a5c6efcfeaa08fa319e4bdf6f942c63d055dd"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_u128"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:u128=c as u128;n==1114111u128}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 124,
            sha256: r###"87b5db4631a75c758e5c431b655d667e8f15e38dbf1492ed2100c5628ac25ccf"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_usize"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:usize=c as usize;n==1114111usize}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 127,
            sha256: r###"a2c56d704aafd42214fd5a9685888dc135f240a72d63d966780ff7852288407c"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_i8"###,
            source: r###"fn run()->bool{let c='\u{80}';let n:i8=c as i8;n==-128i8}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 111,
            sha256: r###"2ea61426fbf509aeed269a5a619d2e6f1528a3a676a9101b525dd65735fb2abc"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_i16"###,
            source: r###"fn run()->bool{let c='\u{ff80}';let n:i16=c as i16;n==-128i16}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 116,
            sha256: r###"688c92feb26e03e26b4253ba1a870a5dc1935bd2f6df1219dff81b34033d52c8"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_i32"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:i32=c as i32;n==1114111i32}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 121,
            sha256: r###"31b95cc9aa53376afdc2e271e964b22dc8ab95f4c7f77df404b38980bdb1745a"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_i64"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:i64=c as i64;n==1114111i64}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 121,
            sha256: r###"7bc4fb0df42f198d74f5f0598178ee8c0fd78fc0bff6128642c14ed1c5e44519"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_i128"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:i128=c as i128;n==1114111i128}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 124,
            sha256: r###"975eac9476464927e4994ec30d0dffa7f777d470498a24ec8356598efd5cb5c3"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"char_to_isize"###,
            source: r###"fn run()->bool{let c='\u{10ffff}';let n:isize=c as isize;n==1114111isize}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 127,
            sha256: r###"73b0c68ac90d33a331d619c52453e444cb1dfd5fa5493730ca7a50e104cdfd76"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"u8_to_char"###,
            source: r###"fn cast(x:u8)->char{x as char}fn run()->bool{cast(0u8)=='\0'&&cast(127u8)=='\u{7f}'&&cast(128u8)=='\u{80}'&&cast(255u8)=='ÿ'}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 180,
            sha256: r###"5847b144264d63cc88dc515778935442f4644fedcfa136f23b3f4d3f131485f3"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"astral_narrowing"###,
            source: r###"fn run()->bool{let c='🦀';(c as u8)==128u8&&(c as i8)==-128i8&&(c as u16)==63872u16&&(c as i16)==-1664i16&&(c as u128)==129408u128&&(c as i128)==129408i128}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 212,
            sha256: r###"d538f960ac06d32b1195fd8ae557c337b6f04d8cf51dde5bf9d3da4582a58d44"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"once_evaluation"###,
            source: r###"fn next(n:&mut i32)->char{*n=*n+1;'🦀'}fn byte(n:&mut i32)->u8{*n=*n+1;255u8}fn run()->bool{let mut n=0;let a=next(&mut n) as u8;let b=byte(&mut n) as char;n==2&&a==128u8&&b=='ÿ'}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 236,
            sha256: r###"4af3bc4af007bab9b6ea1287c57fd2033983221e35bb949d790c2e56c4feee8d"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"generic_aggregate"###,
            source: r###"struct Pair{c:char,n:u128}fn id<T>(x:T)->T{x}fn run()->bool{let p=Pair{c:id(255u8 as char),n:id('🦀' as u128)};let t=(p.c,p.n);let a=[t.0,'a'];a[0usize]=='ÿ'&&t.1==129408u128}fn main(){if run(){println!("char cast oracle ok");}}
"###,
            bytes: 232,
            sha256: r###"93f4cef5e1ccb7bd389152585200d904447cc66d637c328cb0d828de9985e04f"###,
            boundary: Boundary::Live,
            stock_compile_code: 0,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"reject_u16_to_char"###,
            source: r###"fn main(){let n=65u16;let c=n as char;}
"###,
            bytes: 40,
            sha256: r###"f67249266ae9fd7edfad8ad798f6e0708073e5e082b045eaa6a1938a92539f44"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `u16`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_u16_to_char.rs:1:29
  |
1 | fn main(){let n=65u16;let c=n as char;}
  |                             ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_u16_to_char.rs:1:29
  |
1 | fn main(){let n=65u16;let c=n as char;}
  |                             ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_u32_to_char"###,
            source: r###"fn main(){let n=65u32;let c=n as char;}
"###,
            bytes: 40,
            sha256: r###"17d513a4e3814d9c9b816e2559cc186dd7aefd5cccc26551abe7398e26d902e6"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `u32`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_u32_to_char.rs:1:29
  |
1 | fn main(){let n=65u32;let c=n as char;}
  |                             ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead
  |
1 - fn main(){let n=65u32;let c=n as char;}
1 + fn main(){let n=65u32;let c=char::from_u32(n);}
  |

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_u64_to_char"###,
            source: r###"fn main(){let n=65u64;let c=n as char;}
"###,
            bytes: 40,
            sha256: r###"7cb72126855924269fd124e31fb02f8e089243231dc12d9648042b0d44e8b09e"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `u64`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_u64_to_char.rs:1:29
  |
1 | fn main(){let n=65u64;let c=n as char;}
  |                             ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_u64_to_char.rs:1:29
  |
1 | fn main(){let n=65u64;let c=n as char;}
  |                             ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_u128_to_char"###,
            source: r###"fn main(){let n=65u128;let c=n as char;}
"###,
            bytes: 41,
            sha256: r###"c2c3ebcbc7855ae019f896b0b653e8444fcb6ab151f36fd2ef3e6bf245354bc2"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `u128`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_u128_to_char.rs:1:30
  |
1 | fn main(){let n=65u128;let c=n as char;}
  |                              ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_u128_to_char.rs:1:30
  |
1 | fn main(){let n=65u128;let c=n as char;}
  |                              ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_usize_to_char"###,
            source: r###"fn main(){let n=65usize;let c=n as char;}
"###,
            bytes: 42,
            sha256: r###"4215676be05853925d87a7469e7be5e1fb100aa4934d445ed290e8a581552d3a"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `usize`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_usize_to_char.rs:1:31
  |
1 | fn main(){let n=65usize;let c=n as char;}
  |                               ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_usize_to_char.rs:1:31
  |
1 | fn main(){let n=65usize;let c=n as char;}
  |                               ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_i8_to_char"###,
            source: r###"fn main(){let n=65i8;let c=n as char;}
"###,
            bytes: 39,
            sha256: r###"ef960465fb1dfa9058c0cf5aea584f3e990812a8ce98c6d060cc1cd5c04118b4"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `i8`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i8_to_char.rs:1:28
  |
1 | fn main(){let n=65i8;let c=n as char;}
  |                            ^^^^^^^^^ invalid cast
  |
help: consider casting from `u8` instead
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i8_to_char.rs:1:28
  |
1 | fn main(){let n=65i8;let c=n as char;}
  |                            ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_i16_to_char"###,
            source: r###"fn main(){let n=65i16;let c=n as char;}
"###,
            bytes: 40,
            sha256: r###"167ab039fb6aa82a582157b0d3dd5dda592d75bf704161b19dd9f9afd0c61494"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `i16`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i16_to_char.rs:1:29
  |
1 | fn main(){let n=65i16;let c=n as char;}
  |                             ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i16_to_char.rs:1:29
  |
1 | fn main(){let n=65i16;let c=n as char;}
  |                             ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_i32_to_char"###,
            source: r###"fn main(){let n=65i32;let c=n as char;}
"###,
            bytes: 40,
            sha256: r###"b2cae3e81275cf71c2bcd7499ca734bc01f490fb041d2da7a76ca8caa54b81f9"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `i32`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i32_to_char.rs:1:29
  |
1 | fn main(){let n=65i32;let c=n as char;}
  |                             ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i32_to_char.rs:1:29
  |
1 | fn main(){let n=65i32;let c=n as char;}
  |                             ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_i64_to_char"###,
            source: r###"fn main(){let n=65i64;let c=n as char;}
"###,
            bytes: 40,
            sha256: r###"97cf5cb84373237e8914437f3ffee7ebea43c02c2d79b4d65977a1f4dfc24eab"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `i64`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i64_to_char.rs:1:29
  |
1 | fn main(){let n=65i64;let c=n as char;}
  |                             ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i64_to_char.rs:1:29
  |
1 | fn main(){let n=65i64;let c=n as char;}
  |                             ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_i128_to_char"###,
            source: r###"fn main(){let n=65i128;let c=n as char;}
"###,
            bytes: 41,
            sha256: r###"eba47e7f750ccb368873e2ade57ae8f0562f57136942573c3591b607555d7a28"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `i128`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i128_to_char.rs:1:30
  |
1 | fn main(){let n=65i128;let c=n as char;}
  |                              ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_i128_to_char.rs:1:30
  |
1 | fn main(){let n=65i128;let c=n as char;}
  |                              ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_isize_to_char"###,
            source: r###"fn main(){let n=65isize;let c=n as char;}
"###,
            bytes: 42,
            sha256: r###"a9aa0bfd20d29f3887663915f4e3b08c17bfb1cd41675acf1f35f8c63d95328d"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `isize`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_isize_to_char.rs:1:31
  |
1 | fn main(){let n=65isize;let c=n as char;}
  |                               ^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_isize_to_char.rs:1:31
  |
1 | fn main(){let n=65isize;let c=n as char;}
  |                               ^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_char_to_f32"###,
            source: r###"fn main(){let x='a' as f32;}
"###,
            bytes: 29,
            sha256: r###"07f9b0e9210acaef5df608f05be0c274d2952bad133455111c14b22e53416b1f"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0606]: casting `char` as `f32` is invalid
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_char_to_f32.rs:1:17
  |
1 | fn main(){let x='a' as f32;}
  |                 ^^^^^^^^^^
  |
  = help: cast through an integer first

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0606`.
"###,
        },
        Case {
            name: r###"reject_char_to_f64"###,
            source: r###"fn main(){let x='a' as f64;}
"###,
            bytes: 29,
            sha256: r###"665009bb00a911d71659e526a5f15b7449f2c29ad644f445d05903fc8e331f30"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0606]: casting `char` as `f64` is invalid
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_char_to_f64.rs:1:17
  |
1 | fn main(){let x='a' as f64;}
  |                 ^^^^^^^^^^
  |
  = help: cast through an integer first

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0606`.
"###,
        },
        Case {
            name: r###"reject_f32_to_char"###,
            source: r###"fn main(){let x=65f32 as char;}
"###,
            bytes: 32,
            sha256: r###"19105e783b11539f65bdf91ee7d7a05312b16e5de50235dce0ddf88f7f3a156f"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `f32`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_f32_to_char.rs:1:17
  |
1 | fn main(){let x=65f32 as char;}
  |                 ^^^^^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_f32_to_char.rs:1:17
  |
1 | fn main(){let x=65f32 as char;}
  |                 ^^^^^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_f64_to_char"###,
            source: r###"fn main(){let x=65f64 as char;}
"###,
            bytes: 32,
            sha256: r###"207831d47db0dbc21ea0aff4e4811d92719debe216e309aaaa9489af56a4fd9e"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `f64`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_f64_to_char.rs:1:17
  |
1 | fn main(){let x=65f64 as char;}
  |                 ^^^^^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead (via a `u32`)
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_f64_to_char.rs:1:17
  |
1 | fn main(){let x=65f64 as char;}
  |                 ^^^^^^^^^^^^^

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_bool_to_char"###,
            source: r###"fn main(){let x=true as char;}
"###,
            bytes: 31,
            sha256: r###"1a519c6bc9b8d2a2897385869221b775294c87977101ba6a6f2abe82ff440df8"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `bool`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_bool_to_char.rs:1:17
  |
1 | fn main(){let x=true as char;}
  |                 ^^^^^^^^^^^^ invalid cast

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_char_to_bool"###,
            source: r###"fn main(){let x='a' as bool;}
"###,
            bytes: 30,
            sha256: r###"15e8b8abac5c542217f64ef5e5234cb005cbe0e03efe2ccbc6f80f0697a1bef8"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0054]: cannot cast `char` as `bool`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_char_to_bool.rs:1:17
  |
1 | fn main(){let x='a' as bool;}
  |                 ^^^^^^^^^^^ unsupported cast

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0054`.
"###,
        },
        Case {
            name: r###"reject_ref_to_char"###,
            source: r###"fn main(){let x=&65u8 as char;}
"###,
            bytes: 32,
            sha256: r###"076336f2b11d5cab3948d569d64bd4a32477f7e489f87d9bf5252783e1388d3b"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0606]: casting `&u8` as `char` is invalid
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_ref_to_char.rs:1:17
  |
1 | fn main(){let x=&65u8 as char;}
  |                 ^^^^^^^^^^^^^
  |
help: remove the unneeded borrow
  |
1 - fn main(){let x=&65u8 as char;}
1 + fn main(){let x=65u8 as char;}
  |

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0606`.
"###,
        },
        Case {
            name: r###"reject_char_to_ref"###,
            source: r###"fn main(){let x='a' as &u8;}
"###,
            bytes: 29,
            sha256: r###"1312b9ef65a58cb9f625e6410f36c0cbae97cb8872bf1e4e3010342e6b1954a3"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0605]: non-primitive cast: `char` as `&u8`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_char_to_ref.rs:1:17
  |
1 | fn main(){let x='a' as &u8;}
  |                 ^^^^^^^^^^ an `as` expression can only be used to convert between primitive types or to coerce to a specific trait object

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0605`.
"###,
        },
        Case {
            name: r###"reject_char_ref_to_int"###,
            source: r###"fn main(){let x=&'a' as u32;}
"###,
            bytes: 30,
            sha256: r###"5a757a5a215f63dc7cb21ea5b7b9e051079141849655b76a122d361d0de552c7"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0606]: casting `&char` as `u32` is invalid
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_char_ref_to_int.rs:1:17
  |
1 | fn main(){let x=&'a' as u32;}
  |                 ^^^^^^^^^^^
  |
  = help: cast through a raw pointer first

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0606`.
"###,
        },
        Case {
            name: r###"reject_unused_int_to_char"###,
            source: r###"fn unused()->char{65u32 as char}fn main(){}
"###,
            bytes: 44,
            sha256: r###"1b8e1ee6aec0a279c47ab60d96202e8183b24690c40926026ce3d538dd12fb29"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0604]: only `u8` can be cast as `char`, not `u32`
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_unused_int_to_char.rs:1:19
  |
1 | fn unused()->char{65u32 as char}fn main(){}
  |                   ^^^^^^^^^^^^^ invalid cast
  |
help: consider using `char::from_u32` instead
  |
1 - fn unused()->char{65u32 as char}fn main(){}
1 + fn unused()->char{char::from_u32(65u32)}fn main(){}
  |

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0604`.
"###,
        },
        Case {
            name: r###"reject_unused_char_to_float"###,
            source: r###"fn unused()->f64{'a' as f64}fn main(){}
"###,
            bytes: 40,
            sha256: r###"cc12150ac835eb8a3b5faf5937e856ff196af6a69a110147710d22e3943d869c"###,
            boundary: Boundary::Type,
            stock_compile_code: 1,
            stock_compile_stderr: r###"error[E0606]: casting `char` as `f64` is invalid
 --> /home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/reject_unused_char_to_float.rs:1:18
  |
1 | fn unused()->f64{'a' as f64}fn main(){}
  |                  ^^^^^^^^^^
  |
  = help: cast through an integer first

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0606`.
"###,
        },
    ]
}

#[derive(Default)]
struct Failures(Vec<String>);
impl Failures {
    fn cell(&mut self, label: String, f: impl FnOnce()) {
        if let Err(error) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            let detail = error
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| error.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "non-string panic".into());
            self.0.push(format!("{label}: {detail}"));
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} failed cells:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}
fn stock_all() {
    static STOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STOCK.get_or_init(|| {
        let mut failures = Failures::default();
        for case in cases() {
            failures.cell(format!("stock {}", case.name), || {
                let scratch = Scratch::new();
                let input = scratch.source(case.source);
                let output = scratch.path("stock");
                assert_eq!(case.source.len(), case.bytes);
                assert!(case.source.ends_with('\n'));
                let hash = capture(Command::new("/usr/bin/sha256sum").arg(&input), &scratch, 10);
                success(&hash, "source SHA256");
                assert_eq!(
                    String::from_utf8(hash.stdout).unwrap(),
                    format!("{}  {}\n", case.sha256, input.display())
                );
                let result = capture(
                    Command::new("/usr/bin/rustc")
                        .env_remove("RUSTC_BOOTSTRAP")
                        .arg(&input)
                        .args(["--edition=2024", "-A", "warnings"])
                        .arg("-o")
                        .arg(&output),
                    &scratch,
                    30,
                );
                assert_eq!(
                    result.status.code(),
                    Some(case.stock_compile_code),
                    "{result:?}"
                );
                assert!(result.stdout.is_empty());
                if case.boundary == Boundary::Live {
                    assert!(result.stderr.is_empty());
                    runtime(&output, &scratch);
                } else {
                    let expected = case.stock_compile_stderr.replace(
                        &format!(
                            "/home/jeremy/.jcode/scratch/char-casts-stock-20261018/sources/{}.rs",
                            case.name
                        ),
                        &input.display().to_string(),
                    );
                    assert_eq!(String::from_utf8(result.stderr).unwrap(), expected);
                    assert!(!output.exists());
                }
            });
        }
        failures.finish();
    });
}
fn runtime(output: &Path, scratch: &Scratch) {
    let bytes = fs::read(output).unwrap();
    assert!(bytes.len() >= 64);
    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
    let mut command = Command::new(output);
    no_compiler_tools(&mut command, scratch);
    let result = capture(&mut command, scratch, 5);
    assert_eq!(result.status.code(), Some(0), "{result:?}");
    assert_eq!(result.stdout, b"char cast oracle ok\n");
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn rejection(case: &Case, text: &str, graph: bool) {
    assert!(!text.contains("panicked at"), "{}: {text}", case.name);
    if case.boundary == Boundary::ParseUnsupported {
        assert!(
            text.contains("borrowing requires a local-root static place"),
            "{}: {text}",
            case.name
        );
        if graph {
            assert!(
                text.contains("Unsupported") || text.contains("unsupported:"),
                "{}: {text}",
                case.name
            );
        }
        return;
    }
    assert!(!text.contains("Unsupported"), "{}: {text}", case.name);
    let expected = if case.name == "reject_char_ref_to_int" {
        "unsupported cast source"
    } else if matches!(case.name, "reject_char_to_bool" | "reject_char_to_ref") {
        "unsupported cast target"
    } else if case.name.starts_with("reject_char_to_") || case.name == "reject_unused_char_to_float"
    {
        "unsupported cast from char: target must be an integer"
    } else {
        "unsupported cast to char: source must be u8"
    };
    assert!(
        text.contains(expected),
        "{} expected {expected:?}: {text}",
        case.name
    );
    if case.boundary == Boundary::Type && graph {
        assert!(text.contains("InvalidMetadata"), "{}: {text}", case.name);
    }
}
fn no_compiler_tools(command: &mut Command, scratch: &Scratch) {
    command
        .current_dir(&scratch.0)
        .env_remove("RUSTC_BOOTSTRAP")
        .env("RUSTC", scratch.path("no-rustc"))
        .env("PATH", scratch.path("no-tools"));
}
fn isolated(name: &str, marker: &str, run: fn()) {
    if std::env::var_os(marker).is_some() {
        run();
        return;
    }
    stock_all();
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    command
        .args(["--exact", name, "--nocapture"])
        .env(marker, "1");
    let result = capture(&mut command, &scratch, 240);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    success(&result, name);
}
fn successful_graph(case: &Case, compiled: &compiler::Compilation, check: bool) {
    let program = native_prelude::parse(case.source).unwrap();
    frontend::check(&program).unwrap();
    let plan =
        frontend::PlanCtx::from_program(&program, &compiler::ir::PlanBudget::default()).unwrap();
    let expected_bodies = match case.name {
        "u8_to_char" | "generic_aggregate" => 3,
        "once_evaluation" => 4,
        _ => 2,
    };
    let mut source_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        let universal = match plan.callable_provider(declaration).unwrap() {
            frontend::CallableProvider::Source { .. } => {
                source_bodies += usize::from(!plan.is_prelude_decl(declaration).unwrap());
                plan.callable_env(declaration).unwrap().is_generic()
            }
            frontend::CallableProvider::Recipe { .. } => true,
            frontend::CallableProvider::UnavailableProvided { .. } => panic!("unavailable body"),
        };
        for kind in if universal {
            &[110, 111, 112][..]
        } else {
            &[110][..]
        } {
            let key = Key::new(*kind, vec![index as u64]);
            let nodes: Vec<_> = compiled
                .report
                .node_evaluations
                .iter()
                .filter(|(actual, _)| *actual == key)
                .collect();
            assert_eq!(nodes.len(), 1, "{}: {key:?}", case.name);
            assert_eq!(nodes[0].1, 1);
        }
    }
    assert_eq!(
        source_bodies, expected_bodies,
        "{} body inventory",
        case.name
    );
    assert_eq!(compiled.functions, expected_bodies);
    assert_eq!(compiled.source_bytes, case.bytes);
    assert_eq!(compiled.source_files, 1);
    assert_eq!(compiled.report.failures, 0);
    assert_eq!(compiled.report.events_dropped, 0);
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 110)
            .count(),
        plan.callable_count()
    );
    for kind in [102, 108, 109, 103] {
        let nodes: Vec<_> = compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == kind)
            .collect();
        assert_eq!(
            nodes.len(),
            if kind == 103 && check {
                0
            } else {
                compiled.control_flow.len()
            },
            "{}: kind {kind}",
            case.name
        );
        assert!(nodes.iter().all(|(_, count)| *count == 1));
    }
}
#[test]
fn frozen_char_casts_stock_and_frontend_boundaries() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 38);
    assert_eq!(
        all.iter().filter(|c| c.boundary == Boundary::Live).count(),
        16
    );
    assert_eq!(
        all.iter()
            .filter(|c| c.boundary == Boundary::Syntax)
            .count(),
        0
    );
    assert_eq!(
        all.iter().filter(|c| c.boundary == Boundary::Type).count(),
        22
    );
    assert_eq!(
        all.iter()
            .filter(|c| c.boundary == Boundary::ParseUnsupported)
            .count(),
        0
    );
    let mut failures = Failures::default();
    for case in all {
        failures.cell(format!("phase {}", case.name), || {
            if matches!(case.boundary, Boundary::Syntax | Boundary::ParseUnsupported) {
                rejection(
                    &case,
                    &frontend::parse(case.source)
                        .expect_err("syntax accepted")
                        .to_string(),
                    false,
                );
            } else {
                let parsed = frontend::parse(case.source)
                    .expect("must pass parsing, including type-invalid bodies");
                let checked = frontend::check(&parsed);
                if case.boundary == Boundary::Live {
                    checked.expect("positive type check");
                } else {
                    rejection(
                        &case,
                        &checked.expect_err("type-invalid body accepted").to_string(),
                        false,
                    );
                }
            }
        });
    }
    failures.finish();
}
fn cli_all() {
    let mut failures = Failures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.cell(
                    format!("cli {} workers={workers} check={check}", case.name),
                    || {
                        let scratch = Scratch::new();
                        let input = scratch.source(case.source);
                        let output = scratch.path("native");
                        let mut command = compiler(&input, &output, workers, check);
                        no_compiler_tools(&mut command, &scratch);
                        let result = capture(&mut command, &scratch, 30);
                        if case.boundary == Boundary::Live {
                            success(&result, case.name);
                            assert!(result.stderr.is_empty(), "{result:?}");
                            let report = String::from_utf8(result.stdout.clone())
                                .expect("successful CLI returns UTF-8 JSON");
                            let compact: String =
                                report.chars().filter(|c| !c.is_whitespace()).collect();
                            assert!(
                                compact.starts_with('{') && compact.ends_with('}'),
                                "{report}"
                            );
                            for field in [
                                "\"capture_complete\":true".to_owned(),
                                "\"failures\":0".to_owned(),
                                format!("\"workers\":{workers}"),
                                format!("\"source_bytes\":{}", case.bytes),
                            ] {
                                assert!(
                                    compact.contains(&format!("{field},"))
                                        || compact.contains(&format!("{field}}}")),
                                    "{report}"
                                );
                            }
                            assert_eq!(
                                compact.contains("\"executable_bytes\":0,")
                                    || compact.contains("\"executable_bytes\":0}"),
                                check,
                                "{report}"
                            );
                            if check {
                                assert!(!output.exists(), "check published executable");
                            } else {
                                runtime(&output, &scratch);
                                runtimes += 1;
                            }
                        } else {
                            assert!(result.stdout.is_empty(), "{result:?}");
                            assert_eq!(result.status.code(), Some(1), "{result:?}");
                            rejection(&case, &String::from_utf8_lossy(&result.stderr), true);
                            assert!(!output.exists(), "invalid source published executable");
                            assert!(!scratch.path("input").exists());
                            assert!(!scratch.path("a.out").exists());
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 152);
    failures.cell("runtime inventory".into(), || assert_eq!(runtimes, 32));
    failures.finish();
}
#[test]
fn char_casts_cli_workers_one_and_eight_check_build() {
    isolated(
        "char_casts_cli_workers_one_and_eight_check_build",
        "JR_CHAR_CASTS_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn char_casts_observed_graph_workers_one_and_eight_check_build() {
    isolated(
        "char_casts_observed_graph_workers_one_and_eight_check_build",
        "JR_CHAR_CASTS_GRAPH_CHILD",
        graph_all,
    );
}
fn graph_all() {
    let mut failures = Failures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.cell(
                    format!("graph {} workers={workers} check={check}", case.name),
                    || {
                        let scratch = Scratch::new();
                        let input = scratch.source(case.source);
                        let result = compiler::compile_file_observed(input, workers, check);
                        if case.boundary == Boundary::Live {
                            let compiled =
                                result.unwrap_or_else(|error| panic!("{}: {error}", case.name));
                            successful_graph(&case, &compiled, check);
                            assert!(compiled.report.capture_complete);
                            assert_eq!(compiled.executable.is_none(), check);
                            assert!(!compiled.control_flow.is_empty());
                            assert_eq!(compiled.control_flow.len(), compiled.analyses.len());
                            assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
                            for (index, cfg) in compiled.control_flow.iter().enumerate() {
                                cfg.validate().expect("retained CFG validates");
                                for kind in [102, 108, 109] {
                                    let key = Key::new(kind, vec![index as u64]);
                                    let evaluated: Vec<_> = compiled
                                        .report
                                        .node_evaluations
                                        .iter()
                                        .filter(|(actual, _)| *actual == key)
                                        .collect();
                                    assert_eq!(evaluated.len(), 1, "missing or duplicate {key:?}");
                                    assert_eq!(evaluated[0].1, 1);
                                }
                            }
                            if case.name == "u8_to_char" {
                                assert!(
                                    compiled
                                        .control_flow
                                        .iter()
                                        .any(|cfg| cfg.result == compiler::ir::Ty::Char),
                                    "char result type lost"
                                );
                            }
                            if check {
                                assert!(
                                    !compiled
                                        .report
                                        .node_evaluations
                                        .iter()
                                        .any(|(key, _)| key.kind == 103)
                                );
                            }
                            if let Some(image) = compiled.executable {
                                let scratch = Scratch::new();
                                let output = scratch.path("graph-native");
                                fs::write(&output, image).unwrap();
                                fs::set_permissions(&output, fs::Permissions::from_mode(0o700))
                                    .unwrap();
                                runtime(&output, &scratch);
                                runtimes += 1;
                            }
                        } else {
                            let failure = result.err().expect("invalid source accepted");
                            rejection(&case, &failure.to_string(), true);
                            let report = failure.report.expect("failure retains graph");
                            assert!(report.capture_complete);
                            assert_eq!(report.events_dropped, 0);
                            assert!(report.failures > 0);
                            assert!(
                                !report.published_kind(105),
                                "rejected source reached codegen/emission"
                            );
                            assert!(!scratch.path("input").exists());
                            assert!(!scratch.path("a.out").exists());
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, 152);
    failures.cell("runtime inventory".into(), || assert_eq!(runtimes, 32));
    failures.finish();
}
