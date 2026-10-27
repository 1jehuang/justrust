//! Bounded literal panic, not formatting, unwinding, or full Rust support.
//! Frozen original receipt: panic-stock/run-20261018T004613988227Z/results.json.
//! SHA256 cf71725e4b54dba07768094fd7b2ecfddd7c26b3e72f4aefe1c972d99a91fe19.
//! Append receipt SHA256 68656fe8949f83042b3acf09956f514d051791ed65e24c347b041b68717f1925.
//! Stock compiler binary SHA256 c916a85b7d4f52d9650399eebd15c672377933e6c2d8e98215757ab0b9b37136.
//! Append receipt: panic-append/receipt.json. Both measured with /usr/bin/rustc
//! 1.98.1 (48a229cea 2026-09-01), --edition=2024, panic=abort except two
//! separately labeled stock unwind controls. Native expectations are contract
//! assertions, with promotions backed by exact stock/native runtime tests.
//! Do not auto-freeze native failures.
//! Exact original core16: stock Abort10/Live3/Invalid3; bounded native contract
//! Abort9/Live1/Invalid3/earlier-boundary3. Named formatting: 03/04/12 remains
//! unsupported. Scalar formatting promotes 05 after exact stock/native proof.
//! Exact projected7: Live6 and one rigid-projection boundary. Separate historical controls6: Unsupported6.
//! Append18: Abort10/Live6/Invalid2, all independently stock-measured first.
//! Public source/IR mutation and SIGABRT blocked/ignored controls live elsewhere.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
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
    Abort,
    Invalid,
    Unsupported,
}
struct Case {
    name: &'static str,
    group: &'static str,
    source: &'static str,
    source_bytes: usize,
    source_fingerprint: u64,
    source_sha256: &'static str,
    native: Native,
    boundary: &'static str,
    stock_compile_code: i32,
    stock_code: Option<i32>,
    stock_stdout: &'static str,
    stock_stderr: &'static str,
    stock_compile_stderr: &'static str,
    stock_errors: &'static [&'static str],
    message: &'static str,
    location: &'static str,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: r###"01_empty"###,
            group: r###"core"###,
            source: r###"fn main(){ panic!(); }
"###,
            source_bytes: 23,
            source_fingerprint: 6710801254652341751,
            source_sha256: r###"9c5e18b6c79b33f76a9d4e8b4b0f08942822ac2d4f009db7dcb03c293e2f7e39"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940086) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/01_empty/source.rs:1:12:
explicit panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"explicit panic"###,
            location: r###"1:12"###,
        },
        Case {
            name: r###"02_message"###,
            group: r###"core"###,
            source: r###"fn main(){ panic!("message"); }
"###,
            source_bytes: 32,
            source_fingerprint: 2308995648743160908,
            source_sha256: r###"05ea91cc6ae817b3bd6953d444002c0206885358bdb49809f9f946e65623a559"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940158) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/02_message/source.rs:1:12:
message
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"message"###,
            location: r###"1:12"###,
        },
        Case {
            name: r###"03_branch_live"###,
            group: r###"core"###,
            source: r###"fn main(){ let x: i32 = if true { 42 } else { panic!("branch"); }; println!("{x}"); }
"###,
            source_bytes: 86,
            source_fingerprint: 1414309745764810560,
            source_sha256: r###"c3bd04b4e8319558d97ecb6cb55e1523587ef996db5143f29c0b7e01db9159fc"###,
            native: Native::Unsupported,
            boundary: r###"only {} and {:?} formatting placeholders are supported"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"42
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"04_branch_panic"###,
            group: r###"core"###,
            source: r###"fn main(){ let x: i32 = if false { 42 } else { panic!("branch"); }; println!("{x}"); }
"###,
            source_bytes: 87,
            source_fingerprint: 2899173439825184039,
            source_sha256: r###"0057f5f2956c3aeb3b1f30bec6d316b0a11228bbf6a09299ae396d55f39c65c1"###,
            native: Native::Unsupported,
            boundary: r###"only {} and {:?} formatting placeholders are supported"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940260) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/04_branch_panic/source.rs:1:48:
branch
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"branch"###,
            location: r###"1:48"###,
        },
        Case {
            name: r###"05_return_live"###,
            group: r###"core"###,
            source: r###"fn f(b: bool)->i32 { if b { return panic!("return"); } 42 } fn main(){ println!("{}",f(false)); }
"###,
            source_bytes: 98,
            source_fingerprint: 12840476451369224194,
            source_sha256: r###"32229db50972251c63a126c47f6ca67558defe79fd32da102ffd351c31965abe"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"42
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"06_return_panic"###,
            group: r###"core"###,
            source: r###"fn f()->i32 { return panic!("return"); } fn main(){ let _=f(); }
"###,
            source_bytes: 65,
            source_fingerprint: 16986413521008706194,
            source_sha256: r###"b4e93fee537dee5b1b523aec3293152a7e47dc97622ff69532144b5f0830f09d"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940325) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/06_return_panic/source.rs:1:22:
return
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"return"###,
            location: r###"1:22"###,
        },
        Case {
            name: r###"07_initializer"###,
            group: r###"core"###,
            source: r###"fn main(){ let _x: i32 = panic!("initializer"); println!("unreachable"); }
"###,
            source_bytes: 75,
            source_fingerprint: 13031499209699012716,
            source_sha256: r###"0a886377b165b8adcd5bf45100ebd916a0624959dcbfccc14ae02fae5cfb0089"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940359) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/07_initializer/source.rs:1:26:
initializer
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"initializer"###,
            location: r###"1:26"###,
        },
        Case {
            name: r###"08_unreachable_tail"###,
            group: r###"core"###,
            source: r###"fn f()->i32 { panic!("tail"); 42 } fn main(){ let _=f(); }
"###,
            source_bytes: 59,
            source_fingerprint: 2798565930982082814,
            source_sha256: r###"2e8198224aab344f18f9d6ca663a16a63d926f2c11e563d6a0eaf31735597050"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940393) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/08_unreachable_tail/source.rs:1:15:
tail
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"tail"###,
            location: r###"1:15"###,
        },
        Case {
            name: r###"09_unreachable_invalid"###,
            group: r###"core"###,
            source: r###"fn main(){ panic!("before"); let _x:i32=true; }
"###,
            source_bytes: 48,
            source_fingerprint: 4437084673970793978,
            source_sha256: r###"ea9fc1d6fd73e954d1ab51b5636552fb86922e9ebe54c8a0313220e587d12e6f"###,
            native: Native::Invalid,
            boundary: r###"type"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"warning: unreachable statement
 --> /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/09_unreachable_invalid/source.rs:1:30
  |
1 | fn main(){ panic!("before"); let _x:i32=true; }
  |            ----------------  ^^^^^^^^^^^^^^^^ unreachable statement
  |            |
  |            any code following this expression is unreachable
  |
  = note: `#[warn(unreachable_code)]` (part of `#[warn(unused)]`) on by default

error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/09_unreachable_invalid/source.rs:1:41
  |
1 | fn main(){ panic!("before"); let _x:i32=true; }
  |                                     --- ^^^^ expected `i32`, found `bool`
  |                                     |
  |                                     expected due to this

error: aborting due to 1 previous error; 1 warning emitted

For more information about this error, try `rustc --explain E0308`.
"###,
            stock_errors: &[r###"E0308"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"10_unreachable_name"###,
            group: r###"core"###,
            source: r###"fn main(){ panic!("before"); missing_function(); }
"###,
            source_bytes: 51,
            source_fingerprint: 13121633434602588493,
            source_sha256: r###"d5258288cd36d7e6eb8525374859cc6aa28978b264b86c4115b13d5c05a59af7"###,
            native: Native::Invalid,
            boundary: r###"unresolved function"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"warning: unreachable statement
 --> /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/10_unreachable_name/source.rs:1:30
  |
1 | fn main(){ panic!("before"); missing_function(); }
  |            ----------------  ^^^^^^^^^^^^^^^^^^^ unreachable statement
  |            |
  |            any code following this expression is unreachable
  |
  = note: `#[warn(unreachable_code)]` (part of `#[warn(unused)]`) on by default

error[E0425]: cannot find function `missing_function` in this scope
 --> /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/10_unreachable_name/source.rs:1:30
  |
1 | fn main(){ panic!("before"); missing_function(); }
  |                              ^^^^^^^^^^^^^^^^ not found in this scope

error: aborting due to 1 previous error; 1 warning emitted

For more information about this error, try `rustc --explain E0425`.
"###,
            stock_errors: &[r###"E0425"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"11_dead_branch_invalid"###,
            group: r###"core"###,
            source: r###"fn main(){ if false { let _x:i32=true; } else { panic!("else"); } }
"###,
            source_bytes: 68,
            source_fingerprint: 12901857204014496183,
            source_sha256: r###"8b3054e757c4ef2ce0195f774842a98185266f5551a9c63fe417b33b9a0ad4b2"###,
            native: Native::Invalid,
            boundary: r###"type"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/11_dead_branch_invalid/source.rs:1:34
  |
1 | fn main(){ if false { let _x:i32=true; } else { panic!("else"); } }
  |                              --- ^^^^ expected `i32`, found `bool`
  |                              |
  |                              expected due to this

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
            stock_errors: &[r###"E0308"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"12_shortcircuit_skip"###,
            group: r###"core"###,
            source: r###"fn main(){ let a=false && panic!("and"); let b=true || panic!("or"); println!("{a} {b}"); }
"###,
            source_bytes: 92,
            source_fingerprint: 13424131026726461316,
            source_sha256: r###"0e466991326ba820709d87c153af6f1d2741a01e774687fcd7440b7a26f691cf"###,
            native: Native::Unsupported,
            boundary: r###"only {} and {:?} formatting placeholders are supported"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"false true
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"13_shortcircuit_and"###,
            group: r###"core"###,
            source: r###"fn main(){ let _=true && panic!("and"); }
"###,
            source_bytes: 42,
            source_fingerprint: 1473039692106551659,
            source_sha256: r###"291e60a9fdc548314ec45d81db095e4c04e858527a019b47ae81598721975cdc"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940468) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/13_shortcircuit_and/source.rs:1:26:
and
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"and"###,
            location: r###"1:26"###,
        },
        Case {
            name: r###"14_shortcircuit_or"###,
            group: r###"core"###,
            source: r###"fn main(){ let _=false || panic!("or"); }
"###,
            source_bytes: 42,
            source_fingerprint: 12052893831561648764,
            source_sha256: r###"86fb309aa906d3f23c14fda798ac5304ef8e8081e6586303febc3d840db705c7"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940502) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/14_shortcircuit_or/source.rs:1:27:
or
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"or"###,
            location: r###"1:27"###,
        },
        Case {
            name: r###"15_evaluation_order"###,
            group: r###"core"###,
            source: r###"fn mark()->i32 { eprintln!("before"); 1 } fn pair(_:i32,_:i32){} fn main(){ pair(mark(),panic!("argument")); eprintln!("after"); }
"###,
            source_bytes: 131,
            source_fingerprint: 14559607051701409597,
            source_sha256: r###"02078a6b396ae9257267b67af1562c2018f8828178939efe776b5f37e09c363c"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"before

thread 'main' (3940536) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/15_evaluation_order/source.rs:1:89:
argument
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"argument"###,
            location: r###"1:89"###,
        },
        Case {
            name: r###"16_initializer_side_effect"###,
            group: r###"core"###,
            source: r###"fn main(){ let _x:i32={ eprintln!("before"); panic!("initializer"); }; eprintln!("after"); }
"###,
            source_bytes: 93,
            source_fingerprint: 7000963985933209928,
            source_sha256: r###"c5ed4b35ea55891ba375a2021bd53d860e09c75917de8e85a0504ddf451ff965"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"before

thread 'main' (3940594) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/16_initializer_side_effect/source.rs:1:46:
initializer
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"initializer"###,
            location: r###"1:46"###,
        },
        Case {
            name: r###"control_format"###,
            group: r###"control"###,
            source: r###"fn main(){ panic!("value {}",7); }
"###,
            source_bytes: 35,
            source_fingerprint: 14463825328469055243,
            source_sha256: r###"2370cb3bbc862e772788a4e95ff5c60fe2fca6d1d5c269034f741e6e2dd9d5e0"###,
            native: Native::Unsupported,
            boundary: r###"panic! formatting"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940631) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/control_format/source.rs:1:12:
value 7
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"value 7"###,
            location: r###"1:12"###,
        },
        Case {
            name: r###"control_nonliteral"###,
            group: r###"control"###,
            source: r###"fn main(){ let s="message"; panic!(s); }
"###,
            source_bytes: 41,
            source_fingerprint: 727808289668931861,
            source_sha256: r###"08f3767ad6917854bec1f25045f45047b97b337c9e8840e9e043a5a26b441a3c"###,
            native: Native::Unsupported,
            boundary: r###"panic! nonliteral arguments are unsupported"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: format argument must be a string literal
 --> /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/control_nonliteral/source.rs:1:36
  |
1 | fn main(){ let s="message"; panic!(s); }
  |                                    ^
  |
help: you might be missing a string literal to format with
  |
1 | fn main(){ let s="message"; panic!("{}", s); }
  |                                    +++++

error: aborting due to 1 previous error

"###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"control_abort_drop"###,
            group: r###"control"###,
            source: r###"struct D; impl Drop for D { fn drop(&mut self){eprintln!("dropped");} } fn main(){ let _d=D; panic!("drop control"); }
"###,
            source_bytes: 119,
            source_fingerprint: 320947439098334438,
            source_sha256: r###"772a046b47d4afe5407292161ed9492d75a743e2f2bf089e7f4c47428fb1aaeb"###,
            native: Native::Unsupported,
            boundary: r###"unsupported: normal Drop requires a scalar-field struct"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940670) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/control_abort_drop/source.rs:1:94:
drop control
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"drop control"###,
            location: r###"1:94"###,
        },
        Case {
            name: r###"control_unwind_drop"###,
            group: r###"control"###,
            source: r###"struct D; impl Drop for D { fn drop(&mut self){eprintln!("dropped");} } fn main(){ let _d=D; panic!("drop control"); }
"###,
            source_bytes: 119,
            source_fingerprint: 320947439098334438,
            source_sha256: r###"772a046b47d4afe5407292161ed9492d75a743e2f2bf089e7f4c47428fb1aaeb"###,
            native: Native::Unsupported,
            boundary: r###"unsupported: normal Drop requires a scalar-field struct"###,
            stock_compile_code: 0,
            stock_code: Some(101),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940704) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/control_unwind_drop/source.rs:1:94:
drop control
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
dropped
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"drop control"###,
            location: r###"1:94"###,
        },
        Case {
            name: r###"control_catch_abort"###,
            group: r###"control"###,
            source: r###"fn main(){ let r=std::panic::catch_unwind(||panic!("caught control")); println!("caught={}",r.is_err()); }
"###,
            source_bytes: 107,
            source_fingerprint: 12183947067569788194,
            source_sha256: r###"351c760037a6c67c5dc9fbd593d5a2cbbb1c43239a8d77fb5a3089a0f1673cc1"###,
            native: Native::Unsupported,
            boundary: r###"stored closures and capture environments are unsupported"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3940731) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/control_catch_abort/source.rs:1:45:
caught control
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"caught control"###,
            location: r###"1:45"###,
        },
        Case {
            name: r###"control_catch_unwind"###,
            group: r###"control"###,
            source: r###"fn main(){ let r=std::panic::catch_unwind(||panic!("caught control")); println!("caught={}",r.is_err()); }
"###,
            source_bytes: 107,
            source_fingerprint: 12183947067569788194,
            source_sha256: r###"351c760037a6c67c5dc9fbd593d5a2cbbb1c43239a8d77fb5a3089a0f1673cc1"###,
            native: Native::Unsupported,
            boundary: r###"stored closures and capture environments are unsupported"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"caught=true
"###,
            stock_stderr: r###"
thread 'main' (3940765) panicked at /home/jeremy/.jcode/scratch/panic-stock/run-20261018T004613988227Z/control_catch_unwind/source.rs:1:45:
caught control
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"caught control"###,
            location: r###"1:45"###,
        },
        Case {
            name: r###"projected_01_associated_only"###,
            group: r###"projected"###,
            source: r###"trait Source { type Out; }
struct S;
impl Source for S { type Out = i32; }
fn main() { let x: <S as Source>::Out = 42; if x != 42 { panic!(); } println!("ok"); }
"###,
            source_bytes: 162,
            source_fingerprint: 6190151617249631579,
            source_sha256: r###"9f954fc4f36be79a3fa21cf55e1ba5faf9a2cddf242dc741b17bf266a9144970"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"projected_02_closed_required_get"###,
            group: r###"projected"###,
            source: r###"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = i32; fn get(self) -> Self::Out { 42 } }
fn main() { let x: <S as Source>::Out = S.get(); if x != 42 { panic!(); } println!("ok"); }
"###,
            source_bytes: 227,
            source_fingerprint: 12985953376539834374,
            source_sha256: r###"6b4779a47886f525bfb33112e5689f1109d7a7cc90908b34f1f2b35e9bb23d41"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"projected_03_same_self_trait_args"###,
            group: r###"projected"###,
            source: r###"trait Source<A> { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source<i32> for S { type Out = i128; fn get(self) -> Self::Out { 123 } }
impl Source<bool> for S { type Out = bool; fn get(self) -> Self::Out { true } }
fn main() { let a: <S as Source<i32>>::Out = <S as Source<i32>>::get(S); let b: <S as Source<bool>>::Out = <S as Source<bool>>::get(S); if a != 123 || !b { panic!(); } println!("ok"); }
"###,
            source_bytes: 411,
            source_fingerprint: 5117600138922805622,
            source_sha256: r###"adf27966476de500243062396da07e0ee2346f16394f5d7bba0da700522320c1"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"projected_04_noncopy_cell_move"###,
            group: r###"projected"###,
            source: r###"struct Cell { value: i32 }
trait Source { type Out; fn get(self) -> Self::Out; }
struct S(Cell);
impl Source for S { type Out = Cell; fn get(self) -> Self::Out { self.0 } }
fn consume(x: Cell) -> i32 { x.value }
fn main() { let x: <S as Source>::Out = S(Cell { value: 42 }).get(); let moved = x; if consume(moved) != 42 { panic!(); } println!("ok"); }
"###,
            source_bytes: 352,
            source_fingerprint: 7789640676397729382,
            source_sha256: r###"182b63ec1aac669658995121e8b793c05d6cb3217c6a1e86aee052a861dd0da5"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"projected_05_tuple_i128"###,
            group: r###"projected"###,
            source: r###"trait Source { type Out; fn get(self) -> Self::Out; }
struct S;
impl Source for S { type Out = (i128, bool); fn get(self) -> Self::Out { (170141183460469231731687303715884105727, true) } }
fn main() { let x: <S as Source>::Out = S.get(); if x.0 != 170141183460469231731687303715884105727 || !x.1 { panic!(); } println!("ok"); }
"###,
            source_bytes: 328,
            source_fingerprint: 2890756173018844210,
            source_sha256: r###"6b59a59b88970b96366e53282913d1c7d50c9b91f73becc8a879ab7096bcd52b"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"projected_06_generic_impl_wrap"###,
            group: r###"projected"###,
            source: r###"trait Source { type Out; fn get(self) -> Self::Out; }
struct Wrap<T>(T);
impl<T> Source for Wrap<T> { type Out = T; fn get(self) -> Self::Out { self.0 } }
fn main() { let a = Wrap(42i32).get(); let b = Wrap(true).get(); if a != 42 || !b { panic!(); } println!("ok"); }
"###,
            source_bytes: 269,
            source_fingerprint: 14121636504449818243,
            source_sha256: r###"0300cdde291a9a8e5ff8321fda8f6a4bf85a71538d215fa3be7f26ef9f54c0c4"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"projected_07_generic_forward"###,
            group: r###"projected"###,
            source: r###"trait Source { type Out; fn get(self) -> Self::Out; }
struct Wrap<T>(T);
impl<T> Source for Wrap<T> { type Out = T; fn get(self) -> Self::Out { self.0 } }
fn forward<T: Source>(x: T) -> <T as Source>::Out { x.get() }
fn main() { let a = forward(Wrap(42i32)); let b = forward(Wrap(true)); if a != 42 || !b { panic!(); } println!("ok"); }
"###,
            source_bytes: 337,
            source_fingerprint: 4060936354597546651,
            source_sha256: r###"ab87023553b8fd6dfbef14d702725c4862f9c9145de75b76ae5d7bc287920a58"###,
            native: Native::Unsupported,
            boundary: r###"unsupported: rigid generic associated projection"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"branch_abort_literal"###,
            group: r###"append"###,
            source: r###"fn main(){let x:i32=if false {42} else {panic!("branch")}; if x==42 {println!("after");}}
"###,
            source_bytes: 90,
            source_fingerprint: 4869370161783259397,
            source_sha256: r###"9a9549a1370a7687b963c1cda2417c6e6c7b25f3efe8d6806224955ac80de910"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3961578) panicked at /home/jeremy/.jcode/scratch/panic-append/branch_abort_literal/source.rs:1:41:
branch
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"branch"###,
            location: r###"1:41"###,
        },
        Case {
            name: r###"branch_live_predicate"###,
            group: r###"append"###,
            source: r###"fn main(){ let x:i32=if true {42} else {panic!("branch")}; if x==42 {println!("ok");} }
"###,
            source_bytes: 88,
            source_fingerprint: 11388468699878792182,
            source_sha256: r###"d78134698875d31c71d1c31aa9119f70b46a6de94a1f2fd43362ad5e54a7ec64"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"empty_braces"###,
            group: r###"append"###,
            source: r###"fn main(){panic!{};}
"###,
            source_bytes: 21,
            source_fingerprint: 4307428235275788474,
            source_sha256: r###"92a998bcf3f56d9bce359f9647796a2e3e5e91bfee2344a175c8ee39feae384b"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3961649) panicked at /home/jeremy/.jcode/scratch/panic-append/empty_braces/source.rs:1:11:
explicit panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"explicit panic"###,
            location: r###"1:11"###,
        },
        Case {
            name: r###"empty_brackets"###,
            group: r###"append"###,
            source: r###"fn main(){panic![];}
"###,
            source_bytes: 21,
            source_fingerprint: 12003545247363957306,
            source_sha256: r###"9c35fc8a40fe1f2c53ef734583f460154c3c08c63d8114d4f28ddb2880ec50bd"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3961692) panicked at /home/jeremy/.jcode/scratch/panic-append/empty_brackets/source.rs:1:11:
explicit panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"explicit panic"###,
            location: r###"1:11"###,
        },
        Case {
            name: r###"escaped_braces"###,
            group: r###"append"###,
            source: r###"fn main(){panic!("escaped {{braces}} \"quote\"\nline");}
"###,
            source_bytes: 57,
            source_fingerprint: 179758456178150134,
            source_sha256: r###"c8e575c36068c1816cee2b30b14dbb93894e1c736099f6efa38962d4e24ab346"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3961726) panicked at /home/jeremy/.jcode/scratch/panic-append/escaped_braces/source.rs:1:11:
escaped {braces} "quote"
line
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"escaped {braces} "quote"
line"###,
            location: r###"1:11"###,
        },
        Case {
            name: r###"evaluation_once"###,
            group: r###"append"###,
            source: r###"fn mark()->i32 {println!("before");1} fn pair(_:i32,_:i32){} fn main(){pair(mark(),panic!("argument"));println!("after");}
"###,
            source_bytes: 123,
            source_fingerprint: 14216339334277089723,
            source_sha256: r###"5b98074be098ac0acab3a0635d310fea3a843adb44524e0a88b5ee34b3aa5890"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###"before
"###,
            stock_stderr: r###"
thread 'main' (3961764) panicked at /home/jeremy/.jcode/scratch/panic-append/evaluation_once/source.rs:1:84:
argument
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"argument"###,
            location: r###"1:84"###,
        },
        Case {
            name: r###"function_identity"###,
            group: r###"append"###,
            source: r###"fn panic()->i32{42} fn main(){if panic()==42 {println!("ok");}}
"###,
            source_bytes: 64,
            source_fingerprint: 5657381824415972293,
            source_sha256: r###"082b9adc22ba9143b93c4803b88a3b70ef9dc2b57d3f2cac5947b93001169586"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"initializer_once"###,
            group: r###"append"###,
            source: r###"fn main(){let _x:i32={println!("before");panic!("initializer")};println!("after");}
"###,
            source_bytes: 84,
            source_fingerprint: 1722946373997147937,
            source_sha256: r###"22a80746e2deb7537ce3f10d2ce35aa67c4422b20b0122e6126db70d9e29b2e4"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###"before
"###,
            stock_stderr: r###"
thread 'main' (3961902) panicked at /home/jeremy/.jcode/scratch/panic-append/initializer_once/source.rs:1:42:
initializer
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"initializer"###,
            location: r###"1:42"###,
        },
        Case {
            name: r###"let_else_abort"###,
            group: r###"append"###,
            source: r###"enum E { A(i32), B } fn main(){let E::A(x)=E::B else {panic!("let else")};if x==42 {println!("after");}}
"###,
            source_bytes: 105,
            source_fingerprint: 5113843315211514763,
            source_sha256: r###"f6b7308f0c36f45618b33c808ee88ed02e486268d32dd65694cb9de5e8947660"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3961936) panicked at /home/jeremy/.jcode/scratch/panic-append/let_else_abort/source.rs:1:55:
let else
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"let else"###,
            location: r###"1:55"###,
        },
        Case {
            name: r###"let_else_invalid"###,
            group: r###"append"###,
            source: r###"enum E { A(i32), B } fn main(){let E::A(x)=E::B else {42};let _=x;}
"###,
            source_bytes: 68,
            source_fingerprint: 17818152163118393429,
            source_sha256: r###"6bac5577e869e1de04ebce06a5617ae7966b4764f268426efb336991791bd088"###,
            native: Native::Invalid,
            boundary: r###"let-else else block must have type Never and never complete"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error[E0308]: `else` clause of `let...else` does not diverge
 --> /home/jeremy/.jcode/scratch/panic-append/let_else_invalid/source.rs:1:54
  |
1 | enum E { A(i32), B } fn main(){let E::A(x)=E::B else {42};let _=x;}
  |                                                      ^^^^ expected `!`, found integer
  |
  = note: expected type `!`
             found type `{integer}`
  = help: try adding a diverging expression, such as `return` or `panic!(..)`
  = help: ...or use `match` instead of `let...else`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
            stock_errors: &[r###"E0308"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"let_else_live"###,
            group: r###"append"###,
            source: r###"enum E { A(i32), B } fn main(){let E::A(x)=E::A(42) else {panic!("let else")};if x==42 {println!("ok");}}
"###,
            source_bytes: 106,
            source_fingerprint: 6647612824061526225,
            source_sha256: r###"a824138165897d98f9b3d0a7be8783837744b2bab7712de6d56f2890af67b0fd"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"lexical_macro"###,
            group: r###"append"###,
            source: r###"fn main(){macro_rules! panic {()=>{42};} let x:i32=panic!();if x==42 {println!("ok");}}
"###,
            source_bytes: 88,
            source_fingerprint: 4144239607383402515,
            source_sha256: r###"af967862eac7e082fb4b67bcc84a63795adb819ab134f3eb295883984e900d10"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"return_live_predicate"###,
            group: r###"append"###,
            source: r###"fn f(b:bool)->i32 {if b {return panic!("return");} 42} fn main(){if f(false)==42 {println!("ok");}}
"###,
            source_bytes: 100,
            source_fingerprint: 18192329334073346985,
            source_sha256: r###"ec4dd96c9b3f39711be24ee0a40f15fd982ccb6180c81079329489c7c567e139"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"shortcircuit_predicate"###,
            group: r###"append"###,
            source: r###"fn main(){let a=false && panic!("and");let b=true || panic!("or");if !a {if b {println!("ok");}}}
"###,
            source_bytes: 98,
            source_fingerprint: 1555596146769915695,
            source_sha256: r###"d40f89ddfce3d0e2e7aaa5187df319ae4f6b44818a2a891ad7d1d3501e116f78"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"ok
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"trailing_braces"###,
            group: r###"append"###,
            source: r###"fn main(){panic!{"trailing",};}
"###,
            source_bytes: 32,
            source_fingerprint: 2424406120838105680,
            source_sha256: r###"abceb9ec6737d251ffdf1e5294f7edb1ce7737f61da7ba6ddfa1f2e4a1613103"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3962104) panicked at /home/jeremy/.jcode/scratch/panic-append/trailing_braces/source.rs:1:11:
trailing
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"trailing"###,
            location: r###"1:11"###,
        },
        Case {
            name: r###"trailing_brackets"###,
            group: r###"append"###,
            source: r###"fn main(){panic!["trailing",];}
"###,
            source_bytes: 32,
            source_fingerprint: 9481536367136067408,
            source_sha256: r###"72da8d85ced0e3f1784aecaf16b14fcb68e1d8c5872d0328a800b8fef51f1f3e"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3962138) panicked at /home/jeremy/.jcode/scratch/panic-append/trailing_brackets/source.rs:1:11:
trailing
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"trailing"###,
            location: r###"1:11"###,
        },
        Case {
            name: r###"trailing_parens"###,
            group: r###"append"###,
            source: r###"fn main(){panic!("trailing",);}
"###,
            source_bytes: 32,
            source_fingerprint: 12017795462961122443,
            source_sha256: r###"be2d3aeb748593750c3700b2cf06111cb664728c8c544a9316fde543525f68c6"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (3962174) panicked at /home/jeremy/.jcode/scratch/panic-append/trailing_parens/source.rs:1:11:
trailing
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"trailing"###,
            location: r###"1:11"###,
        },
        Case {
            name: r###"unused_generic_invalid"###,
            group: r###"append"###,
            source: r###"fn unused<T>(){panic!("dead");let _x:i32=true;} fn main(){println!("ok");}
"###,
            source_bytes: 75,
            source_fingerprint: 11457722223111789004,
            source_sha256: r###"3c9df9d18bd324a492539fff46fa52e5037f52f5a90dbb7f112b99bb4e19c264"###,
            native: Native::Invalid,
            boundary: r###"type"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"warning: unreachable statement
 --> /home/jeremy/.jcode/scratch/panic-append/unused_generic_invalid/source.rs:1:31
  |
1 | fn unused<T>(){panic!("dead");let _x:i32=true;} fn main(){println!("ok");}
  |                -------------- ^^^^^^^^^^^^^^^^ unreachable statement
  |                |
  |                any code following this expression is unreachable
  |
  = note: `#[warn(unreachable_code)]` (part of `#[warn(unused)]`) on by default

error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/panic-append/unused_generic_invalid/source.rs:1:42
  |
1 | fn unused<T>(){panic!("dead");let _x:i32=true;} fn main(){println!("ok");}
  |                                      --- ^^^^ expected `i32`, found `bool`
  |                                      |
  |                                      expected due to this

error: aborting due to 1 previous error; 1 warning emitted

For more information about this error, try `rustc --explain E0308`.
"###,
            stock_errors: &[r###"E0308"###],
            message: r###""###,
            location: r###""###,
        },
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
            "jr-panic-{}-{nonce}-{}",
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
fn panic_runtime(case: &Case, result: &Captured, source: &Path, stock: bool) {
    assert_eq!(
        result.stdout,
        case.stock_stdout.as_bytes(),
        "{}: later/missing/repeated stdout: {result:?}",
        case.name
    );
    if case.stock_code == Some(-6) {
        assert_eq!(
            result.status.signal(),
            Some(6),
            "{}: SIGABRT required, not exit101/134: {result:?}",
            case.name
        );
        assert_eq!(result.status.code(), None, "{result:?}");
    } else {
        ordinary_exit(
            result,
            case.stock_code.expect("compiled case has stock exit"),
        );
    }
    let stderr = String::from_utf8_lossy(&result.stderr);
    if case.message.is_empty() {
        assert!(stderr.is_empty(), "{}: {result:?}", case.name);
    } else {
        assert!(
            stderr.contains(case.message),
            "{}: missing exact message {:?}: {result:?}",
            case.name,
            case.message
        );
        let location = format!("{}:{}", source.display(), case.location);
        assert!(
            stderr.contains(&location),
            "{}: missing checked invocation {location:?}: {result:?}",
            case.name
        );
        assert!(
            !stderr.lines().any(|line| line == "after"),
            "{}: later stderr executed: {result:?}",
            case.name
        );
        if matches!(
            case.name,
            "15_evaluation_order" | "16_initializer_side_effect"
        ) {
            assert_eq!(stderr.matches("before\n").count(), 1, "{result:?}");
        }
        if stock && case.name == "control_unwind_drop" {
            assert_eq!(stderr.matches("dropped\n").count(), 1);
        }
        if stock && case.name == "control_abort_drop" {
            assert!(!stderr.contains("dropped"));
        }
    }
}
fn runtime(case: &Case, path: &Path, source: &Path, scratch: &Scratch, stock: bool) {
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(Command::new(path).env("RUST_BACKTRACE", "0"), scratch, 5);
    panic_runtime(case, &result, source, stock);
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
            eprintln!("PANIC CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("PANIC CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} panic failures:\n{}",
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
fn frozen_panic_source_and_receipt_partitions() {
    let all = cases();
    assert_eq!(all.len(), 47);
    let mut groups = [0; 4];
    let mut native = [[0; 4]; 4];
    let mut stock_core = [0; 3];
    for case in &all {
        assert_eq!(case.source.len(), case.source_bytes, "{}", case.name);
        assert_eq!(
            fingerprint(case.source),
            case.source_fingerprint,
            "{}",
            case.name
        );
        assert!(case.source.ends_with('\n'));
        assert_eq!(case.source_sha256.len(), 64);
        assert!(case.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        let group = match case.group {
            "core" => 0,
            "projected" => 1,
            "control" => 2,
            "append" => 3,
            _ => panic!("group"),
        };
        groups[group] += 1;
        native[group][match case.native {
            Native::Live => 0,
            Native::Abort => 1,
            Native::Invalid => 2,
            Native::Unsupported => 3,
        }] += 1;
        if group == 0 {
            stock_core[if case.stock_compile_code != 0 {
                2
            } else if case.stock_code == Some(-6) {
                1
            } else {
                0
            }] += 1;
        }
        if case.stock_compile_code != 0 {
            assert_eq!(case.stock_code, None);
            assert!(!case.stock_compile_stderr.is_empty());
            for error in case.stock_errors {
                assert!(case.stock_compile_stderr.contains(error));
            }
        } else {
            assert!(case.stock_code.is_some());
        }
        if case.stock_code == Some(-6) {
            assert!(!case.message.is_empty());
            assert!(!case.location.is_empty());
        }
        if !case.message.is_empty() {
            assert!(case.stock_stderr.contains(case.message));
            assert!(
                case.stock_stderr
                    .contains(&format!("source.rs:{}:", case.location))
            );
        }
    }
    for (group, expected) in [
        ("core", 13332186954905199211),
        ("projected", 10804107170082157633),
        ("control", 17774586114375891887),
        ("append", 7313426972357327822),
    ] {
        let sources: String = all
            .iter()
            .filter(|case| case.group == group)
            .map(|case| case.source)
            .collect();
        assert_eq!(
            fingerprint(&sources),
            expected,
            "{group}: frozen source order/content"
        );
    }
    assert_eq!(groups, [16, 7, 6, 18]);
    assert_eq!(stock_core, [3, 10, 3]);
    assert_eq!(
        native,
        [[1, 9, 3, 3], [6, 0, 0, 1], [0, 0, 0, 6], [6, 10, 2, 0]]
    );
}
#[test]
fn scalar_format_return_live_matches_stock_before_promotion() {
    let case = cases()
        .into_iter()
        .find(|case| case.name == "05_return_live")
        .unwrap();
    let scratch = Scratch::new();
    scratch.write("source.rs", case.source.as_bytes());
    let source = scratch.path("source.rs");
    let stock = scratch.path("stock");
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let built = capture(
        Command::new(rustc)
            .env_remove("RUSTC_BOOTSTRAP")
            .args([
                "--edition=2024",
                "-Cpanic=abort",
                "--crate-name=panic_oracle",
            ])
            .arg(&source)
            .arg("-o")
            .arg(&stock),
        &scratch,
        20,
    );
    ordinary_exit(&built, 0);
    runtime(&case, &stock, &source, &scratch, true);
    for workers in [1, 8] {
        let native = scratch.path(&format!("native-{workers}"));
        let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
        no_compiler_tools(&mut command, &scratch);
        command
            .arg(&source)
            .args(["--workers", &workers.to_string()])
            .arg("-o")
            .arg(&native);
        let built = capture(&mut command, &scratch, 20);
        ordinary_exit(&built, 0);
        runtime(&case, &native, &source, &scratch, false);
    }
}
fn stock_all() {
    static STOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STOCK.get_or_init(stock_all_uncached);
}
fn stock_all_uncached() {
    let scratch = Scratch::new();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(
        Command::new(&rustc)
            .env_remove("RUSTC_BOOTSTRAP")
            .arg("-vV"),
        &scratch,
        10,
    );
    ordinary_exit(&version, 0);
    eprintln!(
        "PANIC ACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut failures = CellFailures::default();
    for case in cases() {
        failures.check(format!("stock {}", case.name), || {
            let scratch = Scratch::new();
            scratch.write("source.rs", case.source.as_bytes());
            let source = scratch.path("source.rs");
            let image = scratch.path("stock");
            let unwind = matches!(case.name, "control_unwind_drop" | "control_catch_unwind");
            let result = capture(
                Command::new(&rustc)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUST_BACKTRACE", "0")
                    .env("LC_ALL", "C")
                    .args([
                        "--edition=2024",
                        "--crate-name=panic_oracle",
                        "--color=never",
                    ])
                    .arg(if unwind {
                        "-Cpanic=unwind"
                    } else {
                        "-Cpanic=abort"
                    })
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
                let text = String::from_utf8_lossy(&result.stderr);
                let codes: Vec<_> = text
                    .lines()
                    .filter_map(|line| line.strip_prefix("error[")?.split_once(']').map(|v| v.0))
                    .collect();
                assert_eq!(codes, case.stock_errors, "{}: {result:?}", case.name);
                if case.name == "control_nonliteral" {
                    assert!(text.contains("format argument must be a string literal"));
                }
            }
        });
    }
    failures.finish();
}
#[test]
fn stock_panic_exact_sources_and_appended_controls() {
    stock_all();
}
fn diagnostic(case: &Case, text: &str) {
    assert!(!text.is_empty(), "{}: empty diagnostic", case.name);
    assert!(
        !text.contains("panicked at"),
        "{}: compiler panic: {text}",
        case.name
    );
    match case.native {
        Native::Invalid => {
            // Ordinary invalid source cannot be laundered into a capability failure.
            assert!(!text.contains("unsupported"), "{}: {text}", case.name);
            if case.boundary == "type" {
                assert!(
                    text.contains("expected") && text.contains("I32") && text.contains("Bool"),
                    "{}: wrong ordinary type error: {text}",
                    case.name
                );
            } else {
                assert!(text.contains(case.boundary), "{}: {text}", case.name);
                if case.name == "10_unreachable_name" {
                    assert!(text.contains("unresolved function"), "{text}");
                }
            }
        }
        Native::Unsupported => {
            // Six historical controls remain unsupported. Their earlier parser/
            // resolution failure is NOT evidence of unwind or formatting support.
            assert!(
                text.contains(case.boundary),
                "{}: expected earlier boundary {:?}: {text}",
                case.name,
                case.boundary
            );
        }
        _ => panic!("{} unexpectedly rejected: {text}", case.name),
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
    frontend::check(&program).expect("all bodies checked independent of reachability");
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
    for &kind in if check {
        &[][..]
    } else {
        &[102u32, 108, 109][..]
    } {
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
                        let result = capture(&mut command, &scratch, 20);
                        let accepted = matches!(case.native, Native::Live | Native::Abort);
                        ordinary_exit(&result, i32::from(!accepted));
                        if !accepted {
                            assert!(result.stdout.is_empty());
                            diagnostic(&case, &String::from_utf8_lossy(&result.stderr));
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
    assert_eq!(cells, 188);
    failures.check("runtime count", || assert_eq!(runtimes, 64));
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
                                    matches!(case.native, Native::Live | Native::Abort),
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
    assert_eq!(cells, 188);
    failures.check("graph runtime count", || assert_eq!(runtimes, 64));
    failures.finish();
}
fn isolated(name: &str, marker: &str, run: fn()) {
    if std::env::var_os(marker).is_some() {
        run();
        return;
    }
    // Stock comes first with the caller's host-portable tool discovery. The
    // native child then has no stock compiler or linker available in PATH.
    stock_all();
    let scratch = Scratch::new();
    let mut command = Command::new(std::env::current_exe().unwrap());
    no_compiler_tools(&mut command, &scratch);
    command
        .args(["--exact", name, "--nocapture"])
        .env(marker, "1");
    let result = capture_bounded(&mut command, &scratch, 240, 8 * 1024 * 1024);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    ordinary_exit(&result, 0);
}
#[test]
fn native_panic_workers1_8_check_build_actual_elf() {
    isolated(
        "native_panic_workers1_8_check_build_actual_elf",
        "JR_PANIC_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_panic_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_panic_workers1_8_check_build_all_body_graph",
        "JR_PANIC_GRAPH_CHILD",
        graph_all,
    );
}
