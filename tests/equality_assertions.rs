//! Bounded assert_eq!/assert_ne! corpus, not full PartialEq support or native measurements.
//! Stock receipt: /home/jeremy/.jcode/scratch/equality-stock/results.json
//! SHA256 bb4467f816b4903327644849e06d34fb6dfefc3f6949453b1b0af026adb2853f
//! /usr/bin/rustc 1.98.1, edition2024, panic=abort, debug-assertions=yes.
//! Original26 frozen and stock executed before native gates; source bytes unchanged.
//! Append1 separately measured: equality-append-stock/results.json SHA256
//! fd408d9dddd1bb02b52879af25e7205d6cd37a5dfd570e80386faeb21026a015
//! Rejection phases corrected from root gate 7276008oo0. Native gates not run by author.
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
    source: &'static str,
    source_bytes: usize,
    source_fingerprint: u64,
    source_sha256: &'static str,
    native: Native,
    boundary: &'static str,
    phase: &'static str,
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
            name: r###"bool_eq"###,
            source: r###"fn main() { assert_eq!(true, true); println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 9304476279449484921,
            source_sha256: r###"a1085e140a920243c4d135660f1cb01d088308fc3926bc09044b4a1d13b9239e"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"bool_ne"###,
            source: r###"fn main() { assert_ne!(false, true); println!("after"); }
"###,
            source_bytes: 58,
            source_fingerprint: 15701236823500080171,
            source_sha256: r###"14550b4456aba946887591ebf75b3f7a2fc45f1bf7e1b42b098ac2f93cdfbecb"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"integer_eq"###,
            source: r###"fn main() { assert_eq!(42, 42); println!("after"); }
"###,
            source_bytes: 53,
            source_fingerprint: 8255969389145935377,
            source_sha256: r###"3576db19abc2b0f4f596d1d6db6cd12b4376d6f62a3aded758b5affd7ca3771a"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"integer_ne"###,
            source: r###"fn main() { assert_ne!(-7, 7); println!("after"); }
"###,
            source_bytes: 52,
            source_fingerprint: 3264369248594140117,
            source_sha256: r###"c92d069fb45b248f4d79f41befc3fc15a03a8cecd41f55a67ec8435dc3f2379e"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"wide_u128"###,
            source: r###"fn main() { assert_eq!(340282366920938463463374607431768211455u128, 340282366920938463463374607431768211455u128); println!("after"); }
"###,
            source_bytes: 135,
            source_fingerprint: 17330462023207355847,
            source_sha256: r###"bc8212cf45c70fb003989b993c182aa9ee62ee917f432a8428698b54090b6335"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"wide_i128"###,
            source: r###"fn main() { assert_ne!(-170141183460469231731687303715884105728i128, 0i128); println!("after"); }
"###,
            source_bytes: 98,
            source_fingerprint: 14683916219084385044,
            source_sha256: r###"4a3212b6f48975bdfabc744b5e2ab35d3e57fc4314e008d090ff946cebfb7859"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"float_f32"###,
            source: r###"fn main() { assert_eq!(1.5f32, 1.5f32); println!("after"); }
"###,
            source_bytes: 61,
            source_fingerprint: 2851487558380171053,
            source_sha256: r###"b9eef39847138ef2deff2f135a510432dfa83dcf754caf7923641c293b205109"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"float_signed_zero"###,
            source: r###"fn main() { assert_eq!(-0.0f64, 0.0f64); println!("after"); }
"###,
            source_bytes: 62,
            source_fingerprint: 14626153935143824334,
            source_sha256: r###"0ba80df4eb4fcc63fc11cef27a3e938a77b3199223cb32812e1b852a8cd81d67"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"float_nan_ne"###,
            source: r###"fn main() { assert_ne!(0.0f64 / 0.0f64, 0.0f64 / 0.0f64); println!("after"); }
"###,
            source_bytes: 79,
            source_fingerprint: 2081272189548750420,
            source_sha256: r###"4e5b8cb21317f18d0cc0a1b5c36572f423988fb22099500f10137957a621af61"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"float_nan_eq_abort"###,
            source: r###"fn main() { assert_eq!(0.0f64 / 0.0f64, 0.0f64 / 0.0f64); println!("after"); }
"###,
            source_bytes: 79,
            source_fingerprint: 13766484668333027287,
            source_sha256: r###"84cbd5d5bbb5c0c8c0edad8a1870345608c08549b424a99260ec3417161a2ec9"###,
            native: Native::Abort,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (401001) panicked at sources/float_nan_eq_abort.rs:1:13:
assertion `left == right` failed
  left: NaN
 right: NaN
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: left == right"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"eq_brackets"###,
            source: r###"fn main() { assert_eq![true, true]; println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 6326671481631175052,
            source_sha256: r###"0838c259fed98cd12e47dce5b6301140065f0c4e8148b0342e50c32075f0fd33"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"ne_braces"###,
            source: r###"fn main() { assert_ne!{1, 2}; println!("after"); }
"###,
            source_bytes: 51,
            source_fingerprint: 7871360504648433124,
            source_sha256: r###"b8827a962d54fcc8c8cee98ea2eacf4af905ca4bdc201ddb8fafe06919014ba3"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"eq_trailing_comma"###,
            source: r###"fn main() { assert_eq!(1, 1,); println!("after"); }
"###,
            source_bytes: 52,
            source_fingerprint: 7897278654842240719,
            source_sha256: r###"c28251f34e60488c093eb61001011dcb94761f464a7c769d1a506ae7f3936349"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"ne_trailing_comma"###,
            source: r###"fn main() { assert_ne!(1, 2,); println!("after"); }
"###,
            source_bytes: 52,
            source_fingerprint: 9188974535960506309,
            source_sha256: r###"5f1ad625cb1ff8add1c4511cf4257aa2ea57fd0ae9b4fd2b40b5da3ddcc1ff0c"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"nested_logical"###,
            source: r###"fn main() { assert_eq!((true && false) || true, true && (false || true)); println!("after"); }
"###,
            source_bytes: 95,
            source_fingerprint: 13536661840500180409,
            source_sha256: r###"4e7a792e5ffa5a2020543ce9806927be368a04bcdbc5147248fbf89e6041ff6e"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"unit_value"###,
            source: r###"fn main() { let value: () = assert_eq!(1, 1); println!("after"); }
"###,
            source_bytes: 67,
            source_fingerprint: 15216561160145303385,
            source_sha256: r###"2b401ca3953079d76cfec44b275b051adfeac8eb29abab65e97cd2a741c9d176"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"warning: unused variable: `value`
 --> sources/unit_value.rs:1:17
  |
1 | fn main() { let value: () = assert_eq!(1, 1); println!("after"); }
  |                 ^^^^^ help: if this is intentional, prefix it with an underscore: `_value`
  |
  = note: `#[warn(unused_variables)]` (part of `#[warn(unused)]`) on by default

warning: 1 warning emitted

"###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"operands_once_order"###,
            source: r###"fn left() -> i32 { println!("left"); 4 } fn right() -> i32 { println!("right"); 4 } fn main() { assert_eq!(left(), right()); println!("after"); }
"###,
            source_bytes: 146,
            source_fingerprint: 4824488813624019652,
            source_sha256: r###"e49b7f2601e1b025ff5bf4caf385f5c86afa543094353c39ca9e87cbbb16d4f5"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"left
right
after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"ne_operands_once_order"###,
            source: r###"fn left() -> i32 { println!("left"); 4 } fn right() -> i32 { println!("right"); 5 } fn main() { assert_ne!(left(), right()); println!("after"); }
"###,
            source_bytes: 146,
            source_fingerprint: 7026526516856971788,
            source_sha256: r###"9e9906f87f8a76c1515aa5108461f24ff5fb91a300a07798c211e0b4bcaf9352"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"left
right
after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"eq_abort"###,
            source: r###"fn main() { assert_eq!(1, 2); println!("after"); }
"###,
            source_bytes: 51,
            source_fingerprint: 15833695406569271866,
            source_sha256: r###"e7a2b094954a53e8dd47fadc5a4c358e1bbf0ce17ee0e7d48c6719cb09898c9a"###,
            native: Native::Abort,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (401311) panicked at sources/eq_abort.rs:1:13:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: left == right"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"ne_abort"###,
            source: r###"fn main() { assert_ne!(true, true); println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 16512269701086780726,
            source_sha256: r###"93fd17513c7e17611f3e78bc69972a9dfc4ef0e68cc0b610f8b5375c051fbef7"###,
            native: Native::Abort,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (401348) panicked at sources/ne_abort.rs:1:13:
assertion `left != right` failed
  left: true
 right: true
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: left != right"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"lexical_shadow"###,
            source: r###"fn main() { macro_rules! assert_eq { ($a:expr, $b:expr) => { println!("shadow") }; } assert_eq!(1, 2); assert_ne!(1, 2); println!("after"); }
"###,
            source_bytes: 142,
            source_fingerprint: 7978254030129490507,
            source_sha256: r###"c1d7e17ee999aa5279f10d964c55ba45ab1a55bb14d55dc2b002b866915077b4"###,
            native: Native::Unsupported,
            boundary: r###"unsupported macro fragment kind"###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"shadow
after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"nonliteral_message"###,
            source: r###"fn main() { let message = "custom"; assert_eq!(1, 1, message); println!("after"); }
"###,
            source_bytes: 84,
            source_fingerprint: 14403378227036358309,
            source_sha256: r###"13589b58ac0f080c7d44e88bc6c0fc799017c62760382ef24b7717713f22d8c3"###,
            native: Native::Unsupported,
            boundary: r###"assert_eq! nonliteral messages are unsupported"###,
            phase: r###"parse"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: format argument must be a string literal
 --> sources/nonliteral_message.rs:1:54
  |
1 | fn main() { let message = "custom"; assert_eq!(1, 1, message); println!("after"); }
  |                                                      ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; assert_eq!(1, 1, "{}", message); println!("after"); }
  |                                                      +++++

error: aborting due to 1 previous error

"###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"format_message"###,
            source: r###"fn main() { assert_ne!(1, 2, "value {}", 1); println!("after"); }
"###,
            source_bytes: 66,
            source_fingerprint: 8264326413192169909,
            source_sha256: r###"e6e2de2a97f94e1f750735ee2b8a124bb4bdd1eda01d5f2be88445085735df98"###,
            native: Native::Unsupported,
            boundary: r###"assert_ne! formatting placeholders are unsupported"###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"reference_boundary"###,
            source: r###"fn main() { assert_eq!(&1, &1); println!("after"); }
"###,
            source_bytes: 53,
            source_fingerprint: 9913753239745323877,
            source_sha256: r###"db45ade450a559baceea62b3c136890716ab0f15ec1a00ebd31e19c445c2f5ba"###,
            native: Native::Unsupported,
            boundary: r###"reference comparisons and arithmetic are unsupported"###,
            phase: r###"check"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"aggregate_boundary"###,
            source: r###"fn main() { assert_eq!((1, 2), (1, 2)); println!("after"); }
"###,
            source_bytes: 61,
            source_fingerprint: 5782802516103353609,
            source_sha256: r###"23a0237a8becfef80a8248835678a02ac215bd331562e6d180d40e4d9b357087"###,
            native: Native::Unsupported,
            boundary: r###"aggregate operators and comparisons are unsupported"###,
            phase: r###"check"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"mixed_type_invalid"###,
            source: r###"fn main() { assert_eq!(true, 1); println!("after"); }
"###,
            source_bytes: 54,
            source_fingerprint: 17137236600886063978,
            source_sha256: r###"835d20650bfd43d10fa8c24469394349c9915f6088db8c2c23c6e44955e5d287"###,
            native: Native::Invalid,
            boundary: r###"equality operands differ: Bool and Int(I32)"###,
            phase: r###"check"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/mixed_type_invalid.rs:1:30
  |
1 | fn main() { assert_eq!(true, 1); println!("after"); }
  |                              ^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
            stock_errors: &[r###"E0308"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"lexical_shadow_tt"###,
            source: r###"fn main() { macro_rules! assert_eq { ($a:tt, $b:tt) => { println!("shadow") }; } assert_eq!(1, 2); assert_ne!(1, 2); println!("after"); }
"###,
            source_bytes: 138,
            source_fingerprint: 11355346665623112237,
            source_sha256: r###"e396b8baa443d35b2a5771caa97f1a7e7c8fa8cbc3a6878e921105d18b9c009c"###,
            native: Native::Live,
            boundary: r###""###,
            phase: r###"parse"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"shadow
after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
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
            "jr-equality_assertion-{}-{nonce}-{}",
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
            stderr.contains(if stock {
                if case.message.ends_with("left == right") {
                    "assertion \x60left == right\x60 failed"
                } else {
                    "assertion \x60left != right\x60 failed"
                }
            } else {
                case.message
            }),
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
            eprintln!("EQUALITY_ASSERTION CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("EQUALITY_ASSERTION CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} equality_assertion failures:\n{}",
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
fn frozen_equality_sources_and_strict_phases() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 27);
    let mut failures = CellFailures::default();
    let mut partition = [0; 4];
    for case in &all {
        partition[match case.native {
            Native::Live => 0,
            Native::Abort => 1,
            Native::Invalid => 2,
            Native::Unsupported => 3,
        }] += 1;
        failures.check(format!("phase {}", case.name), || {
            assert_eq!(case.source.len(), case.source_bytes);
            assert_eq!(fingerprint(case.source), case.source_fingerprint);
            assert_eq!(case.source_sha256.len(), 64);
            assert!(case.source.ends_with('\n'));
            if case.stock_compile_code != 0 {
                assert!(!case.stock_compile_stderr.is_empty());
            }
            if case.native == Native::Abort {
                assert!(
                    case.stock_stderr
                        .contains(&format!(".rs:{}:", case.location))
                );
            }
            let parsed = frontend::parse(case.source);
            if case.phase == "parse" && matches!(case.native, Native::Unsupported | Native::Invalid)
            {
                diagnostic(
                    case,
                    &parsed.expect_err("frozen parse rejection").to_string(),
                );
            } else {
                let program = parsed.expect("must pass frozen parse phase");
                let checked = frontend::check(&program);
                if matches!(case.native, Native::Live | Native::Abort) {
                    checked.expect("positive all-body check");
                } else {
                    diagnostic(
                        case,
                        &checked.expect_err("frozen check rejection").to_string(),
                    );
                }
            }
        });
    }
    assert_eq!(partition, [18, 3, 1, 5]);
    failures.finish();
}
fn stock_all() {
    static STOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STOCK.get_or_init(stock_all_uncached);
}
fn stock_all_uncached() {
    let scratch = Scratch::new();
    let rustc = "/usr/bin/rustc";
    let version = capture(
        Command::new(rustc).env_remove("RUSTC_BOOTSTRAP").arg("-vV"),
        &scratch,
        10,
    );
    ordinary_exit(&version, 0);
    eprintln!(
        "EQUALITY_ASSERTION ACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut failures = CellFailures::default();
    for case in cases() {
        failures.check(format!("stock {}", case.name), || {
            let scratch = Scratch::new();
            scratch.write("source.rs", case.source.as_bytes());
            let source = scratch.path("source.rs");
            let image = scratch.path("stock");
            let result = capture(
                Command::new(rustc)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUST_BACKTRACE", "0")
                    .env("LC_ALL", "C")
                    .args([
                        "--edition=2024",
                        "--crate-name=equality_assertion_oracle",
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
                let text = String::from_utf8_lossy(&result.stderr);
                let codes: Vec<_> = text
                    .lines()
                    .filter_map(|line| line.strip_prefix("error[")?.split_once(']').map(|v| v.0))
                    .collect();
                assert_eq!(codes, case.stock_errors, "{}: {result:?}", case.name);
                if case.name == "nonliteral_message" {
                    assert!(text.contains("format argument must be a string literal"));
                }
            }
        });
    }
    failures.finish();
}
#[test]
fn stock_equality_assertion_exact_sources() {
    stock_all();
}
fn diagnostic(case: &Case, text: &str) {
    assert!(
        matches!(case.native, Native::Invalid | Native::Unsupported),
        "{} unexpectedly rejected: {text}",
        case.name
    );
    assert!(!text.is_empty());
    assert!(!text.contains("panicked at"));
    assert!(
        text.contains(case.boundary),
        "{} expected {:?}: {text}",
        case.name,
        case.boundary
    );
    if case.native == Native::Invalid {
        assert!(!text.contains("unsupported"));
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
    let mut source_bodies = 0;
    for index in 0..plan.callable_count() {
        let declaration = plan.callable_decl(index).unwrap();
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
        plan.callable_count()
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
        .filter(|case| matches!(case.native, Native::Live | Native::Abort))
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
    assert_eq!(cells, cases().len() * 4);
    failures.check("graph runtime count", || {
        assert_eq!(runtimes, expected_runtimes())
    });
    failures.finish();
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
    let result = capture_bounded(&mut command, &scratch, 240, 8 * 1024 * 1024);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    ordinary_exit(&result, 0);
}
#[test]
fn native_equality_assertion_workers1_8_check_build_actual_elf() {
    isolated(
        "native_equality_assertion_workers1_8_check_build_actual_elf",
        "JR_EQUALITY_ASSERTION_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_equality_assertion_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_equality_assertion_workers1_8_check_build_all_body_graph",
        "JR_EQUALITY_ASSERTION_GRAPH_CHILD",
        graph_all,
    );
}
