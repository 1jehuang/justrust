//! Frozen23 guarded matches sources, independently stock measured before native expectations.
//! Receipt: /home/jeremy/.jcode/scratch/guarded-matches-stock-20261018T020026587510Z/results-v3.json
//! Receipt SHA256: 5d3cc8b9951d3a45db7a8db350c37990ef3d00d8c57475e42691efbf843d21af
//! /usr/bin/rustc --edition=2024 -Cpanic=abort, RUSTC_BOOTSTRAP removed.
//! Stock: 15 compile, 8 reject; one compiled case terminates with SIGABRT.
//! Native measured categories: 15 live, 2 unsupported, 6 invalid, 0 syntax.
//! Guard bindings are bounded to Copy values; non-Copy bindings are explicitly unsupported.
//! The prior21 sources/receipts are untouched. promoted_guard is byte-identical
//! to prior21 guard (5875421359eba37b761e61cddb3b94a904ecce723b5df8d0b06e15950ff17edb),
//! deliberately promoted here from Unsupported to Live. Root owns prior21 promotion.
//! Original bytes remeasured stock/native workers1/8 CHECK/BUILD/run on 2026-10-19.
//! nested_tuple now matches stock exactly; binding_scope rejects unresolved local x.
//! moved_scrutinee remains a non-Copy guard boundary, not move-diagnostic parity.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use stock_rustc::stock_rustc;
#[path = "support/native_prelude.rs"]
mod native_prelude;

use justrust_engine::compiler::{self, frontend, ir};
use justrust_engine::{Key, Report};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Native {
    Live,
    Unsupported,
    Invalid,
    // Partition slot retained after range_pattern became Live; no frozen case uses it now.
    #[allow(dead_code)]
    Syntax,
}
struct Case {
    name: &'static str,
    source: &'static str,
    source_bytes: usize,
    source_fingerprint: u64,
    source_sha256: &'static str,
    native: Native,
    boundary: &'static str,
    phase: &'static str,
    stock_compile_code: i32,
    stock_code: Option<i32>,
    stock_signal: Option<i32>,
    stock_stdout: &'static str,
    stock_stderr: &'static str,
    stock_compile_stderr: &'static str,
}
#[rustfmt::skip]
fn cases() -> Vec<Case> {
 vec![
Case { name: r###"promoted_guard"###, source: r###"fn main(){ if matches!(1, x if x == 1) { println!("guard"); } }
"###, source_bytes: 64, source_fingerprint: 13549839185063572285, source_sha256: r###"5875421359eba37b761e61cddb3b94a904ecce723b5df8d0b06e15950ff17edb"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"guard
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"false_guard"###, source: r###"fn main(){ if matches!(1, x if x == 2) { println!("bad"); } else { println!("false"); } }
"###, source_bytes: 90, source_fingerprint: 4446411092657569854, source_sha256: r###"f0aac1b1d727b07253bd12dff69c055c8413a3c503d2890e3abc93305eba820a"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"false
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"mismatch_side_effect"###, source: r###"fn guard()->bool { println!("bad"); true } fn main(){ if matches!(1, 2 if guard()) { println!("bad"); } println!("skipped"); }
"###, source_bytes: 127, source_fingerprint: 8924946345689971954, source_sha256: r###"50c47341532a5e6aaf11cd2bf4b71bc83d8e5879cc3c2b8495ef561c907d0b36"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"skipped
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"exact_once"###, source: r###"fn value()->i32 { println!("value"); 7 } fn guard(x:i32)->bool { println!("guard"); x == 7 } fn main(){ if matches!(value(), x if guard(x)) { println!("hit"); } }
"###, source_bytes: 163, source_fingerprint: 14182613411339641967, source_sha256: r###"2568bfb8632a9d6c2743e4f32d10b57e523fa2f7f17c938f2096370b4dce4408"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"value
guard
hit
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"nested_tuple"###, source: r###"fn main(){ let x:i32=9; if matches!((1, (2, true)), (x, (y, yes)) if x + y == 3 && yes) { println!("tuple"); } println!("{}", x); }
"###, source_bytes: 132, source_fingerprint: 10757279220051887453, source_sha256: r###"e036289f035a637392ebbf242efdedf363161af281910418b45c0b609746e73b"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"tuple
9
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"nested_enum"###, source: r###"enum E { A((i32, bool)), B } fn main(){ if matches!(E::A((4, true)), E::A((x, yes)) if x == 4 && yes) { println!("enum"); } if matches!(E::B, E::A((_, _)) if panic!("bad")) { println!("bad"); } }
"###, source_bytes: 196, source_fingerprint: 795103591775647741, source_sha256: r###"809203f72393252d53b02b02ee51ed69780e9d038d57a22cabad808824730f32"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"enum
"###, stock_stderr: r###""###, stock_compile_stderr: r###"warning: unreachable expression
 --> sources/nested_enum.rs:1:128
  |
1 | ...ln!("enum"); } if matches!(E::B, E::A((_, _)) if panic!("bad")) { println!("bad"); } }
  |                      ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^-------------^
  |                      |                              |
  |                      |                              any code following this expression is unreachable
  |                      unreachable expression
  |
  = note: `#[warn(unreachable_code)]` (part of `#[warn(unused)]`) on by default

warning: 1 warning emitted

"### },
Case { name: r###"delimiters_trailing"###, source: r###"fn main(){ if matches!(1, x if x == 1) { println!("p"); } if matches!(1, x if x == 1,) { println!("pt"); } if matches![1, x if x == 1] { println!("b"); } if matches![1, x if x == 1,] { println!("bt"); } if matches!{1, x if x == 1} { println!("c"); } if matches!{1, x if x == 1,} { println!("ct"); } }
"###, source_bytes: 301, source_fingerprint: 6297788893407400643, source_sha256: r###"ecf00e29675628f1de92438d43361b4c97d5984eea21878078dc457e25571195"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"p
pt
b
bt
c
ct
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"panic_skipped"###, source: r###"fn main(){ if matches!(1, 2 if panic!("guard-stop")) { println!("bad"); } println!("safe"); }
"###, source_bytes: 94, source_fingerprint: 11459589698696524576, source_sha256: r###"5c105639bfc89eba8388c7e29a0a4d1ed6f14be6d7ce0f1c5b3f4c4a58795e86"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"safe
"###, stock_stderr: r###""###, stock_compile_stderr: r###"warning: unreachable expression
 --> sources/panic_skipped.rs:1:15
  |
1 | fn main(){ if matches!(1, 2 if panic!("guard-stop")) { println!("bad"); } println!("safe"); }
  |               ^^^^^^^^^^^^^^^^^--------------------^
  |               |                |
  |               |                any code following this expression is unreachable
  |               unreachable expression
  |
  = note: `#[warn(unreachable_code)]` (part of `#[warn(unused)]`) on by default

warning: 1 warning emitted

"### },
Case { name: r###"panic_taken"###, source: r###"fn main(){ println!("before"); let _hit=matches!(1, 1 if panic!("guard-stop")); println!("after"); }
"###, source_bytes: 101, source_fingerprint: 2046603907127979978, source_sha256: r###"a13e93a33cd9f0fb31398ddc4eccf9c57573ebd33b78b078da8617b35191feab"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: None, stock_signal: Some(6), stock_stdout: r###"before
"###, stock_stderr: r###"
thread 'main' (<PID>) panicked at <SOURCE>:1:58:
guard-stop
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###, stock_compile_stderr: r###"warning: unreachable expression
 --> sources/panic_taken.rs:1:41
  |
1 | fn main(){ println!("before"); let _hit=matches!(1, 1 if panic!("guard-stop")); println!("after"); }
  |                                         ^^^^^^^^^^^^^^^^^--------------------^
  |                                         |                |
  |                                         |                any code following this expression is unreachable
  |                                         unreachable expression
  |
  = note: `#[warn(unreachable_code)]` (part of `#[warn(unused)]`) on by default

warning: 1 warning emitted

"### },
Case { name: r###"nonbool"###, source: r###"fn main(){ let _hit=matches!(1, _ if 7); }
"###, source_bytes: 43, source_fingerprint: 11101163176140557295, source_sha256: r###"4413ee7d4e3789e55a29a46c253990c96c8ed63e16d3bbb5a788cb07a52c5a7d"###, native: Native::Invalid, boundary: r###""###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/nonbool.rs:1:38
  |
1 | fn main(){ let _hit=matches!(1, _ if 7); }
  |                                      ^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"### },
Case { name: r###"dead_invalid"###, source: r###"fn main(){ if false { let _hit=matches!(1, _ if 7); } }
"###, source_bytes: 56, source_fingerprint: 16472707607143652873, source_sha256: r###"9ea1f36c970ff9ae60f5f7f0713f66962e617c90ea0183e1feab3b546fd02784"###, native: Native::Invalid, boundary: r###""###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/dead_invalid.rs:1:49
  |
1 | fn main(){ if false { let _hit=matches!(1, _ if 7); } }
  |                                                 ^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"### },
Case { name: r###"unused_invalid"###, source: r###"fn unused(){ let _hit=matches!(1, _ if 7); } fn main(){}
"###, source_bytes: 57, source_fingerprint: 1722642062573910950, source_sha256: r###"633bb32529ac3999216cf9bfa2511f8f05937336be2c4fb3a426144ea7f10999"###, native: Native::Invalid, boundary: r###""###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/unused_invalid.rs:1:40
  |
1 | fn unused(){ let _hit=matches!(1, _ if 7); } fn main(){}
  |                                        ^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"### },
Case { name: r###"binding_scope"###, source: r###"fn main(){ let _hit=matches!(1, x if x == 1); println!("{}", x); }
"###, source_bytes: 67, source_fingerprint: 14129497231029623708, source_sha256: r###"35be6abf6f758887d75015313beea58f194df3e7f632f4788af24ef755f340e3"###, native: Native::Invalid, boundary: r###""###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0425]: cannot find value `x` in this scope
 --> sources/binding_scope.rs:1:62
  |
1 | fn main(){ let _hit=matches!(1, x if x == 1); println!("{}", x); }
  |                                                              ^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"### },
Case { name: r###"dead_unknown"###, source: r###"fn main(){ if false { let _hit=matches!(1, _ if missing); } }
"###, source_bytes: 62, source_fingerprint: 10091640673889860466, source_sha256: r###"225614913ead267a2149ad9a881c41e3667da848dfa2b5dbbcac6f822968de2c"###, native: Native::Invalid, boundary: r###""###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0425]: cannot find value `missing` in this scope
 --> sources/dead_unknown.rs:1:49
  |
1 | fn main(){ if false { let _hit=matches!(1, _ if missing); } }
  |                                                 ^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"### },
Case { name: r###"local_shadow"###, source: r###"fn main(){ macro_rules! matches { ($($x:tt)*) => { true }; } if matches!(not a pattern if missing) { println!("local"); } }
"###, source_bytes: 124, source_fingerprint: 12680186938598325009, source_sha256: r###"8093ec4a57b52c83983bd3b355ac27d3234cbee3565c4214a9cbf4420eade064"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"local
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"or_pattern"###, source: r###"fn main(){ if matches!(2, 1 | 2 if true) { println!("or"); } }
"###, source_bytes: 63, source_fingerprint: 3432924276943927410, source_sha256: r###"b95c2072702910516131ea2eb026d4f76585c62c600401f353bed7fb7857e52e"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"or
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"at_pattern"###, source: r###"fn main(){ if matches!(2, x @ 2 if x == 2) { println!("at"); } }
"###, source_bytes: 65, source_fingerprint: 13653741935893286849, source_sha256: r###"9182deb4c379fdcd4de38542fefb0e73082ca1ac51fadf59d7e8fd64b1bfbcae"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"at
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"range_pattern"###, source: r###"fn main(){ if matches!(2, 1..=3 if true) { println!("range"); } }
"###, source_bytes: 66, source_fingerprint: 18395706359714326618, source_sha256: r###"0e2ee45cdcb8a4a03748ef3cda4b21ad07ba8708092a9e47a8117869554e3e36"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"range
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"ordinary_guard"###, source: r###"fn main(){ match 1 { x if x == 1 => println!("ordinary"), _ => () } }
"###, source_bytes: 70, source_fingerprint: 7241983642770566222, source_sha256: r###"710afcfefcec6c25be050920549d708f02246b4a12830425687b8f0444a36f42"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"ordinary
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"moved_scrutinee"###, source: r###"struct S { value:i32 } fn main(){ let s=S { value:1 }; let _hit=matches!(s, x if x.value == 1); println!("{}", s.value); }
"###, source_bytes: 123, source_fingerprint: 17245422516474392823, source_sha256: r###"6509c35e96c03042aaf2abd9560440602163a2d2b3e8a247891a043355789194"###, native: Native::Unsupported, boundary: r###"guarded matches with non-Copy bindings are unsupported"###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0382]: borrow of moved value: `s`
 --> sources/moved_scrutinee.rs:1:112
  |
1 | struct S { value:i32 } fn main(){ let s=S { value:1 }; let _hit=matches!(s, x if x.value == 1); println!("{}", s.value); }
  |                                       -                                     -                                  ^^^^^^^ value borrowed here after move
  |                                       |                                     |
  |                                       |                                     value moved here
  |                                       move occurs because `s` has type `S`, which does not implement the `Copy` trait
  |
help: borrow this binding in the pattern to avoid moving the value
  |
1 | struct S { value:i32 } fn main(){ let s=S { value:1 }; let _hit=matches!(s, ref x if x.value == 1); println!("{}", s.value); }
  |                                                                             +++

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0382`.
"### },
Case { name: r###"noncopy_guard_move"###, source: r###"struct S { value:i32 } enum E { A(S), B } fn consume(s:S)->bool { s.value == 1 } fn main(){ let e=E::A(S { value:1 }); let _hit=matches!(e, E::A(s) if consume(s)); }
"###, source_bytes: 166, source_fingerprint: 2368198729769403704, source_sha256: r###"67898247228aeedf1b1dc0c896d795861bbee032843f36eabda33a1ebf9de4e5"###, native: Native::Unsupported, boundary: r###"guarded matches with non-Copy bindings are unsupported"###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0507]: cannot move out of `s` in pattern guard
 --> sources/noncopy_guard_move.rs:1:160
  |
1 | ...!(e, E::A(s) if consume(s)); }
  |                            ^ move occurs because `s` has type `S`, which does not implement the `Copy` trait
  |
  = note: variables bound in patterns cannot be moved from until after the end of the pattern guard
note: if `S` implemented `Clone`, you could clone the value
 --> sources/noncopy_guard_move.rs:1:1
  |
1 | struct S { value:i32 } enum E { A(S), B } fn consume(s:S)->bool { s.value == 1 } fn main(){ let e=E::A(S { value:1 }); let _hit=matches!(e, E::A(s) if consume(s)); }
  | ^^^^^^^^ consider implementing `Clone` for this type                                                                                                           - you could clone this value

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0507`.
"### },
Case { name: r###"nested_tuple_literal"###, source: r###"fn main(){ let x:i32=9; if matches!((1, (2, true)), (x, (y, yes)) if x + y == 3 && yes) { println!("tuple"); } if x == 9 { println!("outer"); } else { println!("bad"); } }
"###, source_bytes: 172, source_fingerprint: 13771992849515178808, source_sha256: r###"50defefd9361316b9f0b75f831930a00e6c0298bc522d02e0be29aaef0265bce"###, native: Native::Live, boundary: r###""###, phase: r###"check"###, stock_compile_code: 0, stock_code: Some(0), stock_signal: None, stock_stdout: r###"tuple
outer
"###, stock_stderr: r###""###, stock_compile_stderr: r###""### },
Case { name: r###"binding_scope_literal"###, source: r###"fn main(){ let _hit=matches!(1, x if x == 1); let _escaped=x; }
"###, source_bytes: 64, source_fingerprint: 1962071269287578889, source_sha256: r###"07a7efd481a51235c72babbf4669772a501439b78c746212413b5e9adcd5dff2"###, native: Native::Invalid, boundary: r###""###, phase: r###"check"###, stock_compile_code: 1, stock_code: None, stock_signal: None, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0425]: cannot find value `x` in this scope
 --> sources/binding_scope_literal.rs:1:60
  |
1 | fn main(){ let _hit=matches!(1, x if x == 1); let _escaped=x; }
  |                                                            ^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"### },
 ]
}
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = base.join(format!(
            "jr-guarded_matches-{}-{nonce}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) {
        new_file(&self.path(name)).write_all(bytes).unwrap();
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .write(true)
        .create_new(true)
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
// Linux x86_64 ABI, matching this test's target gate. This links the libc already
// used by std and adds no dependency. Only setrlimit is called after fork.
#[repr(C)]
struct Rlimit {
    current: u64,
    maximum: u64,
}
unsafe extern "C" {
    fn setrlimit(resource: std::ffi::c_int, limit: *const Rlimit) -> std::ffi::c_int;
    fn prctl(option: std::ffi::c_int, ...) -> std::ffi::c_int;
}
fn disable_core(command: &mut Command) {
    // SAFETY: the closure only uses stack data, a libc syscall and last_os_error.
    unsafe {
        command.pre_exec(|| {
            let limit = Rlimit {
                current: 0,
                maximum: 0,
            };
            // A piped core_pattern can invoke an external dumper even with
            // RLIMIT_CORE=0. Keep abort observations independent of that service.
            if setrlimit(4, &limit) == 0 && prctl(4, 0usize, 0usize, 0usize, 0usize) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    capture_bounded(command, scratch, seconds, 65536)
}
fn capture_bounded(command: &mut Command, scratch: &Scratch, seconds: u64, limit: u64) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    disable_core(command);
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    let mut retries = 0;
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            Err(error) if error.raw_os_error() == Some(26) && retries < 20 => {
                retries += 1;
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("spawn {command:?}: {error}"),
        }
    };
    let mut child = Running(child);
    let start = Instant::now();
    let status = loop {
        for path in [&out, &err] {
            assert!(
                fs::metadata(path).unwrap().len() <= limit,
                "output bound: {command:?}"
            );
        }
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(fs::metadata(path).unwrap().len() <= limit);
    }
    Captured {
        status,
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn ordinary_exit(result: &Captured, code: i32) {
    assert_eq!(result.status.signal(), None, "{result:?}");
    assert_eq!(result.status.code(), Some(code), "{result:?}");
}
fn no_compiler_tools(command: &mut Command, scratch: &Scratch) {
    command
        .current_dir(&scratch.0)
        .env_remove("RUSTC_BOOTSTRAP")
        .env("RUSTC", scratch.path("no-rustc"))
        .env("PATH", scratch.path("no-tools"));
}
// Only the stock panic PID and temporary source path are normalized.
// Native's deliberately smaller panic diagnostic is compared byte for byte.
fn exact_runtime(case: &Case, result: &Captured, source: &Path, stock: bool) {
    assert_eq!(
        result.status.signal(),
        case.stock_signal,
        "{}: {result:?}",
        case.name
    );
    assert_eq!(
        result.status.code(),
        case.stock_code,
        "{}: {result:?}",
        case.name
    );
    assert_eq!(
        result.stdout,
        case.stock_stdout.as_bytes(),
        "{}: {result:?}",
        case.name
    );
    let mut stderr = String::from_utf8(result.stderr.clone()).unwrap();
    stderr = stderr.replace(&source.display().to_string(), "<SOURCE>");
    let expected = if stock || case.stock_signal.is_some() {
        if let Some(start) = stderr.find("thread 'main' (") {
            let start = start + "thread 'main' (".len();
            let end = start + stderr[start..].find(')').unwrap();
            assert!(stderr[start..end].bytes().all(|b| b.is_ascii_digit()));
            stderr.replace_range(start..end, "<PID>");
        }
        case.stock_stderr.to_owned()
    } else {
        case.stock_stderr.to_owned()
    };
    assert_eq!(stderr, expected, "{} stderr", case.name);
}
fn runtime(case: &Case, path: &Path, source: &Path, scratch: &Scratch, stock: bool) {
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(Command::new(path).env("RUST_BACKTRACE", "0"), scratch, 5);
    exact_runtime(case, &result, source, stock);
}
#[derive(Default)]
struct CellFailures(Vec<String>);
impl CellFailures {
    fn check(&mut self, label: impl Into<String>, check: impl FnOnce()) {
        let label = label.into();
        if let Err(payload) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(check)) {
            let message = payload
                .downcast_ref::<String>()
                .map(String::as_str)
                .or_else(|| payload.downcast_ref::<&str>().copied())
                .unwrap_or("non-string panic payload");
            let failure = format!("{label}: {message}");
            eprintln!("GUARDED_MATCHES CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("GUARDED_MATCHES CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} guarded_matches failures:\n{}",
            self.0.len(),
            self.0.join("\n")
        );
    }
}
fn fingerprint(source: &str) -> u64 {
    source.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}
#[test]
fn frozen_guarded_matches_sources_and_strict_phases() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 23);
    let mut failures = CellFailures::default();
    let mut partition = [0; 4];
    for case in &all {
        partition[match case.native {
            Native::Live => 0,
            Native::Unsupported => 1,
            Native::Invalid => 2,
            Native::Syntax => 3,
        }] += 1;
        failures.check(format!("phase {}", case.name), || {
            assert_eq!(case.source.len(), case.source_bytes);
            assert_eq!(fingerprint(case.source), case.source_fingerprint);
            assert_eq!(case.source_sha256.len(), 64);
            assert!(case.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()));
            assert!(case.source.ends_with('\n'));
            if case.stock_compile_code != 0 {
                assert!(!case.stock_compile_stderr.is_empty());
            }
            if case.phase == "parse" {
                diagnostic(
                    case,
                    &frontend::parse(case.source).expect_err("parse boundary"),
                );
                return;
            }
            let checked = frontend::parse(case.source)
                .and_then(|program| frontend::check(&program).map(|_| ()));
            match case.native {
                Native::Live => checked.expect("positive all-body check"),
                _ => diagnostic(case, &checked.expect_err("frozen rejection")),
            }
            assert!(matches!(case.phase, "parse" | "check"));
        });
    }
    // range_pattern became Live with SCALAR-PATTERNS.md and ordinary_guard
    // with general match guards (sources unchanged).
    assert_eq!(partition, [15, 2, 6, 0]);
    failures.finish();
}
fn stock_all() {
    static STOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STOCK.get_or_init(stock_all_uncached);
}
fn stock_all_uncached() {
    let scratch = Scratch::new();
    let rustc = stock_rustc();
    let rustc = rustc.as_str();
    let version = capture(
        Command::new(rustc).env_remove("RUSTC_BOOTSTRAP").arg("-vV"),
        &scratch,
        10,
    );
    ordinary_exit(&version, 0);
    eprintln!(
        "GUARDED_MATCHES ACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut failures = CellFailures::default();
    for case in cases() {
        failures.check(format!("stock {}", case.name), || {
            let scratch = Scratch::new();
            scratch.write("source.rs", case.source.as_bytes());
            let source = scratch.path("source.rs");
            let image = scratch.path("stock");
            let hash = capture(
                Command::new("/usr/bin/sha256sum").arg(&source),
                &scratch,
                10,
            );
            ordinary_exit(&hash, 0);
            assert!(hash.stderr.is_empty());
            assert_eq!(
                String::from_utf8(hash.stdout).unwrap(),
                format!("{}  {}\n", case.source_sha256, source.display())
            );
            let result = capture(
                Command::new(rustc)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUST_BACKTRACE", "0")
                    .env("LC_ALL", "C")
                    .args([
                        "--edition=2024",
                        "--crate-name=guarded_matches_oracle",
                        "--color=never",
                    ])
                    .arg("-Cpanic=abort")
                    .arg("-Cdebug-assertions=yes")
                    .arg(&source)
                    .arg("-o")
                    .arg(&image),
                &scratch,
                20,
            );
            ordinary_exit(&result, case.stock_compile_code);
            assert!(result.stdout.is_empty(), "{result:?}");
            if case.stock_compile_code == 0 {
                runtime(&case, &image, &source, &scratch, true);
            } else {
                assert!(!image.exists());
            }
            let normalized = String::from_utf8(result.stderr.clone()).unwrap().replace(
                &source.display().to_string(),
                &format!("sources/{}.rs", case.name),
            );
            assert_eq!(
                normalized, case.stock_compile_stderr,
                "{} stock diagnostics",
                case.name
            );
        });
    }
    failures.finish();
}
#[test]
fn stock_guarded_matches_exact_sources() {
    stock_all();
}
// Session identity is process-local. Preserve every other diagnostic byte.
fn normalize_session_ids(text: &str) -> String {
    let mut result = String::new();
    let mut remaining = text;
    while let Some(start) = remaining.find("SessionId(") {
        let end_prefix = start + "SessionId(".len();
        result.push_str(&remaining[..end_prefix]);
        remaining = &remaining[end_prefix..];
        let digits = remaining.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0 && remaining[digits..].starts_with(')') {
            result.push_str("<SESSION>");
            remaining = &remaining[digits..];
        }
    }
    result.push_str(remaining);
    result
}
fn diagnostic(case: &Case, text: &str) {
    assert!(!text.is_empty(), "{}: empty diagnostic", case.name);
    assert!(!text.contains("panicked at"), "{}: {text}", case.name);
    match case.native {
        Native::Live => panic!("{} unexpectedly rejected: {text}", case.name),
        Native::Unsupported => {
            if case.name == "at_pattern" {
                assert!(text.contains("\"@\""), "{text}");
            }
            assert!(text.contains("unsupported"), "{}: {text}", case.name);
            assert!(
                text.contains(case.boundary),
                "{} expected {:?}: {text}",
                case.name,
                case.boundary
            );
        }
        Native::Syntax => {
            assert!(!text.contains("unsupported"), "{}: {text}", case.name);
            assert!(
                text.contains(case.boundary),
                "{} expected {:?}: {text}",
                case.name,
                case.boundary
            );
        }
        Native::Invalid => {
            assert!(!text.contains("unsupported"), "{}: {text}", case.name);
            match case.name {
                "nonbool" | "dead_invalid" | "unused_invalid" => {
                    assert!(
                        text.contains("Bool") && text.contains("I32"),
                        "{}: {text}",
                        case.name
                    );
                }
                "binding_scope" | "binding_scope_literal" => {
                    assert!(text.contains("unresolved local x"), "{text}")
                }
                "dead_unknown" => assert!(text.contains("missing"), "{text}"),
                "moved_scrutinee" => {
                    assert!(text.to_ascii_lowercase().contains("mov"), "{text}")
                }
                _ => panic!("unclassified invalid fixture"),
            }
        }
    }
}
fn evaluated_once(report: &Report, kind: u32, index: usize, case: &Case) {
    let key = Key::new(kind, vec![index as u64]);
    let nodes: Vec<_> = report
        .node_evaluations
        .iter()
        .filter(|(actual, _)| *actual == key)
        .collect();
    assert_eq!(nodes.len(), 1, "{}: missing/duplicate {key:?}", case.name);
    assert_eq!(nodes[0].1, 1, "{}: repeated {key:?}", case.name);
}
fn successful_all_body_graph(case: &Case, compiled: &compiler::Compilation, check: bool) {
    let program = native_prelude::parse(case.source).expect("positive frozen source parses");
    // Public CHECK/BUILD already checked every body through canonical source
    // capabilities. The legacy frontend-only checker cannot authorize embedded
    // prelude receiver inference. The graph counts below verify all-body demand.
    let plan = frontend::PlanCtx::from_program(&program, &ir::PlanBudget::default()).unwrap();
    assert_eq!(compiled.source_bytes, case.source.len());
    assert_eq!(compiled.source_files, 1);
    assert_eq!(compiled.executable.is_none(), check);
    assert_eq!(compiled.report.failures, 0);
    assert_eq!(compiled.report.events_dropped, 0);
    assert!(compiled.report.capture_complete);
    let mut eager_bodies = 0usize;
    let mut source_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
        // Embedded prelude bodies are admitted on demand, never eagerly.
        if plan.is_prelude_decl(declaration).unwrap() {
            continue;
        }
        eager_bodies += 1;
        evaluated_once(&compiled.report, 110, index, case);
        let universal = match plan.callable_provider(declaration).unwrap() {
            frontend::CallableProvider::Source { .. } => {
                source_bodies += usize::from(!plan.is_prelude_decl(declaration).unwrap());
                plan.callable_env(declaration).unwrap().is_generic()
            }
            frontend::CallableProvider::Recipe { .. } => true,
            frontend::CallableProvider::UnavailableProvided { .. } => {
                panic!("unavailable body admitted")
            }
        };
        if universal {
            evaluated_once(&compiled.report, 111, index, case);
            evaluated_once(&compiled.report, 112, index, case);
        }
    }
    assert_eq!(compiled.functions, source_bodies);
    assert_eq!(
        compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == 110)
            .count(),
        eager_bodies
    );
    assert_eq!(compiled.control_flow.len(), compiled.analyses.len());
    assert!(compiled.analyses.iter().all(|analysis| analysis.passed()));
    for kind in [102, 108, 109] {
        let nodes: Vec<_> = compiled
            .report
            .node_evaluations
            .iter()
            .filter(|(key, _)| key.kind == kind)
            .collect();
        assert_eq!(
            nodes.len(),
            compiled.control_flow.len(),
            "{}:{kind}",
            case.name
        );
        assert!(nodes.iter().all(|(_, count)| *count == 1));
    }
    let objects: Vec<_> = compiled
        .report
        .node_evaluations
        .iter()
        .filter(|(key, _)| key.kind == 103)
        .collect();
    assert_eq!(
        objects.len(),
        if check {
            0
        } else {
            compiled.control_flow.len()
        },
        "{}: codegen only in BUILD",
        case.name
    );
    assert!(objects.iter().all(|(_, count)| *count == 1));
}
fn expected_runtimes() -> usize {
    cases()
        .iter()
        .filter(|case| matches!(case.native, Native::Live))
        .count()
        * 2
}
fn cli_all() {
    let mut failures = CellFailures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "CLI {} workers{workers} check{check} {:?}",
                        case.name, case.native
                    ),
                    || {
                        let scratch = Scratch::new();
                        scratch.write("source.rs", case.source.as_bytes());
                        let source = scratch.path("source.rs");
                        let image = scratch.path("native");
                        let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                        no_compiler_tools(&mut command, &scratch);
                        command
                            .arg(&source)
                            .args(["--workers", &workers.to_string()]);
                        if check {
                            command.arg("--check");
                        } else {
                            command.arg("-o").arg(&image);
                        }
                        let result = capture(&mut command, &scratch, 180);
                        let accepted = matches!(case.native, Native::Live);
                        ordinary_exit(&result, i32::from(!accepted));
                        if !accepted {
                            assert!(result.stdout.is_empty());
                            diagnostic(&case, &String::from_utf8_lossy(&result.stderr));
                            let failure = match compiler::compile_file_observed(source.clone(), workers, check) {
                                Err(failure) => failure,
                                Ok(_) => panic!("rejected CLI case unexpectedly accepted by graph"),
                            };
                            let expected = format!("jr-rust: {}: {failure}\nnote: source preparation and syntax diagnostics use original file locations; semantic diagnostic source spans remain incomplete\n", source.display());
                            assert_eq!(normalize_session_ids(&String::from_utf8(result.stderr.clone()).unwrap()), normalize_session_ids(&expected));
                        }
                        if accepted {
                            assert!(result.stderr.is_empty(), "{result:?}");
                            let summary = String::from_utf8(result.stdout.clone()).unwrap();
                            assert!(summary.starts_with("{\"schema\":1,"));
                            assert!(summary.ends_with("\"measurement\":\"contended/uncontrolled\"}\n"));
                            assert!(summary.contains(&format!("\"mode\":\"{}\"", if check { "check" } else { "build" })));
                            assert!(summary.contains(&format!("\"workers\":{workers},")));
                            assert!(summary.contains(&format!("\"source_bytes\":{},", case.source.len())));
                            assert!(summary.contains("\"failures\":0,"));
                            assert!(summary.contains("\"capture_complete\":true,"));
                        }
                        if accepted && !check {
                            runtime(&case, &image, &source, &scratch, false);
                            runtimes += 1;
                        } else {
                            for name in ["native", "source", "a.out"] {
                                assert!(
                                    !scratch.path(name).exists(),
                                    "{}: unexpected artifact {name}",
                                    case.name
                                );
                            }
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, cases().len() * 4);
    failures.check("runtime count", || {
        assert_eq!(runtimes, expected_runtimes())
    });
    failures.finish();
}
fn graph_all() {
    let mut failures = CellFailures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for workers in [1, 8] {
            for check in [true, false] {
                failures.check(
                    format!(
                        "graph {} workers{workers} check{check} {:?}",
                        case.name, case.native
                    ),
                    || {
                        let scratch = Scratch::new();
                        scratch.write("source.rs", case.source.as_bytes());
                        let source = scratch.path("source.rs");
                        match compiler::compile_file_observed(source.clone(), workers, check) {
                            Ok(compiled) => {
                                assert!(
                                    matches!(case.native, Native::Live),
                                    "{} unexpectedly accepted",
                                    case.name
                                );
                                successful_all_body_graph(&case, &compiled, check);
                                if !check {
                                    let path = scratch.path("graph-native");
                                    scratch.write(
                                        "graph-native",
                                        compiled.executable.as_ref().unwrap(),
                                    );
                                    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                                        .unwrap();
                                    runtime(&case, &path, &source, &scratch, false);
                                    runtimes += 1;
                                }
                            }
                            Err(failure) => {
                                diagnostic(&case, &failure.to_string());
                                let report =
                                    failure.report.expect("failure retains observed graph");
                                assert!(report.capture_complete);
                                assert_eq!(report.events_dropped, 0);
                                assert!(report.failures > 0);
                                assert!(
                                    !report.published_kind(105),
                                    "{}: rejected source emitted object",
                                    case.name
                                );
                            }
                        }
                    },
                );
                cells += 1;
            }
        }
    }
    assert_eq!(cells, cases().len() * 4);
    failures.check("graph runtime count", || {
        assert_eq!(runtimes, expected_runtimes())
    });
    failures.finish();
}

#[test]
fn promoted_guarded_or_frozen_workers1_8() {
    stock_all();
    let case = cases()
        .into_iter()
        .find(|case| case.name == "or_pattern")
        .unwrap();
    assert!(matches!(case.native, Native::Live));
    for workers in [1, 8] {
        for check in [true, false] {
            let scratch = Scratch::new();
            scratch.write("source.rs", case.source.as_bytes());
            let source = scratch.path("source.rs");
            let compiled = compiler::compile_file_observed(source.clone(), workers, check)
                .unwrap_or_else(|failure| panic!("workers={workers} check={check}: {failure}"));
            successful_all_body_graph(&case, &compiled, check);
            if !check {
                let image = scratch.path("graph-native");
                scratch.write("graph-native", compiled.executable.as_ref().unwrap());
                fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
                runtime(&case, &image, &source, &scratch, false);
            } else {
                assert!(compiled.executable.is_none());
            }
        }
    }
}

fn isolated(name: &str, marker: &str, run: fn()) {
    if std::env::var_os(marker).is_some() {
        run();
        return;
    }
    // Stock comes first using the installed /usr/bin/rustc. The
    // native child then has no stock compiler or linker available in PATH.
    stock_all();
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    command
        .args(["--exact", name, "--nocapture"])
        .env(marker, "1");
    let result = capture_bounded(&mut command, &scratch, 900, 8 * 1024 * 1024);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    ordinary_exit(&result, 0);
}
#[test]
fn native_guarded_matches_workers1_8_check_build_actual_elf() {
    isolated(
        "native_guarded_matches_workers1_8_check_build_actual_elf",
        "JR_GUARDED_MATCHES_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_guarded_matches_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_guarded_matches_workers1_8_check_build_all_body_graph",
        "JR_GUARDED_MATCHES_GRAPH_CHILD",
        graph_all,
    );
}
