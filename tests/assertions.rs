//! Bounded assert! contracts, not native measurements or full Rust support.
//! Original30 receipt: assert-stock/results.json SHA256
//! abe72fe96d60ac6c54c0bde71f4cb2d4231dede83b8cd7e5fa6abc7a69d06ffd.
//! Append3 receipt: assert-stock/comma-results.json SHA256
//! b4fd962dcc56c97042573f430dd17bb7f093a05a22b27a890d5856ba05692849.
//! Receipts under /home/jeremy/.jcode/scratch, stock /usr/bin/rustc 1.98.1.
//! edition2024, panic=abort, debug-assertions=yes. Frozen before native gates.
//! Default diagnostic meaning, not stock expression-text identity, is required.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
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
            name: r###"true_paren"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(true); println!("after"); }
"###,
            source_bytes: 48,
            source_fingerprint: 7799814017834679446,
            source_sha256: r###"d3b726bdf4bf718d94302f47e67fabecbbf5d4d88b6e21747516d066b3371614"###,
            native: Native::Live,
            boundary: r###""###,
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
            name: r###"false_paren"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(false); println!("after"); }
"###,
            source_bytes: 49,
            source_fingerprint: 6518757341005502915,
            source_sha256: r###"6f9206e971561bb4438172cd9eddec8f7e079aa1f0acc77e774a27134fc4b22b"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (148206) panicked at sources/false_paren.rs:1:13:
assertion failed: false
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: false"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"true_bracket"###,
            group: r###"core"###,
            source: r###"fn main() { assert![true]; println!("after"); }
"###,
            source_bytes: 48,
            source_fingerprint: 7757820239829457743,
            source_sha256: r###"1c1958f497c6821744be08d50ec721eb4fc2458d4b985518a88f6bea5c09a21c"###,
            native: Native::Live,
            boundary: r###""###,
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
            name: r###"false_bracket"###,
            group: r###"core"###,
            source: r###"fn main() { assert![false]; println!("after"); }
"###,
            source_bytes: 49,
            source_fingerprint: 16983106851707403032,
            source_sha256: r###"59914b67bb3784a55430e35fbbf301576abdafc35a5eadfade9ef4207846087a"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (148314) panicked at sources/false_bracket.rs:1:13:
assertion failed: false
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: false"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"true_brace"###,
            group: r###"core"###,
            source: r###"fn main() { assert!{true}; println!("after"); }
"###,
            source_bytes: 48,
            source_fingerprint: 15788495558265211919,
            source_sha256: r###"8a497d862a9298b4d0ce97a4dceb23ae74ffe44af0c2b5774fae4b4de372739a"###,
            native: Native::Live,
            boundary: r###""###,
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
            name: r###"false_brace"###,
            group: r###"core"###,
            source: r###"fn main() { assert!{false}; println!("after"); }
"###,
            source_bytes: 49,
            source_fingerprint: 3788274395241206360,
            source_sha256: r###"c323955fc9009ec2370a611c16c82d25e0eb7c2163a0c5ca98dd179a89cde713"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (148386) panicked at sources/false_brace.rs:1:13:
assertion failed: false
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: false"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"condition_once_true"###,
            group: r###"core"###,
            source: r###"fn hit(n: &mut i32) -> bool { *n += 1; println!("hit"); true } fn main() { let mut n = 0; assert!(hit(&mut n)); if n == 1 { println!("once"); } else { println!("wrong"); } }
"###,
            source_bytes: 174,
            source_fingerprint: 5197140791714501458,
            source_sha256: r###"76a6c9fd1ee975b0e39b791ace4c28b02c7ea5fdeb807758d2b2165923933488"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"hit
once
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"condition_once_false"###,
            group: r###"core"###,
            source: r###"fn hit(n: &mut i32) -> bool { *n += 1; if *n == 1 { println!("hit_once"); } else { println!("wrong"); } false } fn main() { let mut n = 0; assert!(hit(&mut n)); println!("after"); }
"###,
            source_bytes: 182,
            source_fingerprint: 5285328387847075614,
            source_sha256: r###"03dc6acf2c71abc318f631d6c7211967f46da5f7cf6aa930791320d7730d3743"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###"hit_once
"###,
            stock_stderr: r###"
thread 'main' (148462) panicked at sources/condition_once_false.rs:1:140:
assertion failed: hit(&mut n)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: hit(&mut n)"###,
            location: r###"1:140"###,
        },
        Case {
            name: r###"shortcircuit_or"###,
            group: r###"core"###,
            source: r###"fn rhs() -> bool { println!("rhs"); false } fn main() { assert!(true || rhs()); println!("after"); }
"###,
            source_bytes: 101,
            source_fingerprint: 2301278088111185649,
            source_sha256: r###"0b82cd27ba6b7924baa636fba1e471226e451b9fba8f0d6fdae34cb76b4715a5"###,
            native: Native::Live,
            boundary: r###""###,
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
            name: r###"shortcircuit_and"###,
            group: r###"core"###,
            source: r###"fn rhs() -> bool { println!("rhs"); true } fn main() { assert!(false && rhs()); println!("after"); }
"###,
            source_bytes: 101,
            source_fingerprint: 4864337790126223393,
            source_sha256: r###"a1ac0b8d27b4dcfb8a008cea5519e487dab3ebba3c41a38cf52ebf5776b65c49"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (148544) panicked at sources/shortcircuit_and.rs:1:56:
assertion failed: false && rhs()
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: false && rhs()"###,
            location: r###"1:56"###,
        },
        Case {
            name: r###"shortcircuit_rhs_once"###,
            group: r###"core"###,
            source: r###"fn rhs() -> bool { println!("rhs"); true } fn main() { assert!(false || rhs()); println!("after"); }
"###,
            source_bytes: 101,
            source_fingerprint: 15280222382086222077,
            source_sha256: r###"76aa4c8c7673b6579031b51cd6c2b7921fcde3dcda017725e6ecd69c9734e7b2"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"rhs
after
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"condition_panic"###,
            group: r###"core"###,
            source: r###"fn condition() -> bool { println!("condition"); panic!("condition panic") } fn main() { assert!(condition()); println!("after"); }
"###,
            source_bytes: 131,
            source_fingerprint: 578716024268659407,
            source_sha256: r###"e65011834832be72de05bbf75be06c1a0a55a6eacee76ee5dc96659977352767"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###"condition
"###,
            stock_stderr: r###"
thread 'main' (148622) panicked at sources/condition_panic.rs:1:49:
condition panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"condition panic"###,
            location: r###"1:49"###,
        },
        Case {
            name: r###"literal_custom_true"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(true, "custom message"); println!("after"); }
"###,
            source_bytes: 66,
            source_fingerprint: 3179733362134036494,
            source_sha256: r###"3a3bb4d0e080b0e3c5d3565f5d923b5cdfc1a115329af7e5414765d825369cb6"###,
            native: Native::Live,
            boundary: r###""###,
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
            name: r###"literal_custom_false"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(false, "custom message"); println!("after"); }
"###,
            source_bytes: 67,
            source_fingerprint: 6439775095263271403,
            source_sha256: r###"260b36174ff87ecb39019cc3d6349b6dd3398fdd54bae5a157fce55fb332f1e7"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (148723) panicked at sources/literal_custom_false.rs:1:13:
custom message
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"custom message"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"literal_escaped_braces"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(false, "literal {{braces}}"); }
"###,
            source_bytes: 52,
            source_fingerprint: 16697116901920191084,
            source_sha256: r###"13c1cb00692ce3d25539984c391425f126c158c87bfb892b13d338593209b871"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (148778) panicked at sources/literal_escaped_braces.rs:1:13:
literal {braces}
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"literal {braces}"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"literal_non_ascii"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(false, "échec 雪 🦀"); }
"###,
            source_bytes: 49,
            source_fingerprint: 17722469823495547985,
            source_sha256: r###"03531231dc167fbc56841a896630df92a7a7e93744386b956d37aad121e7de63"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (148878) panicked at sources/literal_non_ascii.rs:1:13:
échec 雪 🦀
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"échec 雪 🦀"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"unit_result"###,
            group: r###"core"###,
            source: r###"fn unit(_: ()) { println!("unit"); } fn main() { unit(assert!(true)); }
"###,
            source_bytes: 72,
            source_fingerprint: 14357266126871013664,
            source_sha256: r###"24dbbec8b024c98ea78dd37ea1cc67ebb0147e6240bd3cdfdc1dda4b88ce65cd"###,
            native: Native::Live,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"unit
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"if_branch"###,
            group: r###"core"###,
            source: r###"fn main() { if true { assert!(true) } else { () }; println!("after"); }
"###,
            source_bytes: 72,
            source_fingerprint: 12916807776225552247,
            source_sha256: r###"2317b4ca5c80e3218d82f49e38032f792c45563858031f13f0bcebb496f14a03"###,
            native: Native::Live,
            boundary: r###""###,
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
            name: r###"let_initializer"###,
            group: r###"core"###,
            source: r###"fn main() { let result: () = assert!(true); let _: () = result; println!("after"); }
"###,
            source_bytes: 85,
            source_fingerprint: 5561303990161457174,
            source_sha256: r###"2aa072477f2f94f9fa3b4fc8ae4296aedde2c64fa60e4a4843f16c8755cea14a"###,
            native: Native::Live,
            boundary: r###""###,
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
            name: r###"invalid_non_bool"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(7); }
"###,
            source_bytes: 26,
            source_fingerprint: 12173728299192657371,
            source_sha256: r###"3bff55ce2534f9d528c7bac9732e11c89a11e184cc739a85f8d7215d0a4a8de2"###,
            native: Native::Invalid,
            boundary: r###"bool_type"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/invalid_non_bool.rs:1:13
  |
1 | fn main() { assert!(7); }
  |             ^^^^^^^^^^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
            stock_errors: &[r###"E0308"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"invalid_dead_non_bool"###,
            group: r###"core"###,
            source: r###"fn main() { if false { assert!(7); } }
"###,
            source_bytes: 39,
            source_fingerprint: 5920920788097773729,
            source_sha256: r###"7395457fa6d98cf0f411e5998a4ee6f642515557c7294fe7b6274255313e4f1b"###,
            native: Native::Invalid,
            boundary: r###"bool_type"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/invalid_dead_non_bool.rs:1:24
  |
1 | fn main() { if false { assert!(7); } }
  |                        ^^^^^^^^^^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
            stock_errors: &[r###"E0308"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"invalid_dead_name"###,
            group: r###"core"###,
            source: r###"fn main() { if false { assert!(missing_name); } }
"###,
            source_bytes: 50,
            source_fingerprint: 8751775406659856084,
            source_sha256: r###"7a81d6904a23687a25f821ed16e2cbd5f0b5a3a45743e35590de1f7ba6bbb629"###,
            native: Native::Invalid,
            boundary: r###"missing_name"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error[E0425]: cannot find value `missing_name` in this scope
 --> sources/invalid_dead_name.rs:1:32
  |
1 | fn main() { if false { assert!(missing_name); } }
  |                                ^^^^^^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"###,
            stock_errors: &[r###"E0425"###],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"lexical_shadow_before"###,
            group: r###"core"###,
            source: r###"fn main() { macro_rules! assert { ($e:expr) => { println!("shadow") }; } assert!(false); println!("after"); }
"###,
            source_bytes: 110,
            source_fingerprint: 13823337796330751864,
            source_sha256: r###"c6449caca0c7b2bca50ec3a1f42577030789673ce956abf53188620861ff5d34"###,
            native: Native::Unsupported,
            boundary: r###"unsupported macro fragment kind"###,
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
            name: r###"lexical_shadow_after"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(true); macro_rules! assert { ($e:expr) => { println!("shadow") }; } assert!(false); println!("after"); }
"###,
            source_bytes: 125,
            source_fingerprint: 4171389741908241265,
            source_sha256: r###"ed1dadf5d47cbe9eee1c670a2e75e9f45bf2f22509e3a8062a36e6d4b55c294f"###,
            native: Native::Unsupported,
            boundary: r###"unsupported macro fragment kind"###,
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
            name: r###"module_macro_shadow"###,
            group: r###"core"###,
            source: r###"mod inner { macro_rules! assert { ($e:expr) => { println!("module shadow") }; } pub fn run() { assert!(false); } } fn main() { inner::run(); }
"###,
            source_bytes: 143,
            source_fingerprint: 18085575626397160728,
            source_sha256: r###"421f636b8c529c86b860bc0f6113d1f33766a97f36943eff596939cae530ca02"###,
            native: Native::Unsupported,
            boundary: r###"unsupported macro fragment kind"###,
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"module shadow
"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"deferred_format_argument"###,
            group: r###"core"###,
            source: r###"fn main() { assert!(false, "value {}", 7); }
"###,
            source_bytes: 45,
            source_fingerprint: 10406089384949838343,
            source_sha256: r###"313b64dacb13abc3a4212a5f1cea39d7d97b2b77c9bb8ce85e0bfb214e49725a"###,
            native: Native::Unsupported,
            boundary: r###"assert! formatting placeholders are unsupported"###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (149355) panicked at sources/deferred_format_argument.rs:1:13:
value 7
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"value 7"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"deferred_nonliteral_message"###,
            group: r###"core"###,
            source: r###"fn main() { let message = "custom"; assert!(false, message); }
"###,
            source_bytes: 63,
            source_fingerprint: 921804843175273415,
            source_sha256: r###"5b2ee6845f34ee4af819f91d8750b393e24a87e56cc8747cc639e75764b1d7fe"###,
            native: Native::Unsupported,
            boundary: r###"assert! nonliteral messages are unsupported"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: format argument must be a string literal
 --> sources/deferred_nonliteral_message.rs:1:52
  |
1 | fn main() { let message = "custom"; assert!(false, message); }
  |                                                    ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; assert!(false, "{}", message); }
  |                                                    +++++

error: aborting due to 1 previous error

"###,
            stock_errors: &[],
            message: r###""###,
            location: r###""###,
        },
        Case {
            name: r###"deferred_assert_eq"###,
            group: r###"core"###,
            source: r###"fn main() { assert_eq!(1, 2); }
"###,
            source_bytes: 32,
            source_fingerprint: 183170034912364134,
            source_sha256: r###"af1558356f25a6b49dc26e190e80e6d47894746e81658852a57a92714e3f3221"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (149422) panicked at sources/deferred_assert_eq.rs:1:13:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion `left == right` failed"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"deferred_debug_assert"###,
            group: r###"core"###,
            source: r###"fn main() { debug_assert!(false); }
"###,
            source_bytes: 36,
            source_fingerprint: 4804096277573192797,
            source_sha256: r###"edc9f927f6f1a42b773dd542f586915914060e8187428ead83bfafa5b0371565"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (149471) panicked at sources/deferred_debug_assert.rs:1:13:
assertion failed: false
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: false"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"deferred_format_not_evaluated"###,
            group: r###"core"###,
            source: r###"fn arg() -> i32 { println!("arg"); 7 } fn main() { assert!(true, "value {}", arg()); println!("after"); }
"###,
            source_bytes: 106,
            source_fingerprint: 10099125556028923527,
            source_sha256: r###"b15f173b46463cab3c7ebcac88670b63c3cdc3c81ca0b787ba9255e6f5d4f65c"###,
            native: Native::Unsupported,
            boundary: r###"assert! formatting placeholders are unsupported"###,
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
            name: r###"default_trailing_comma"###,
            group: r###"append"###,
            source: r###"fn main() { assert!(false,); }
"###,
            source_bytes: 31,
            source_fingerprint: 17410252830738371069,
            source_sha256: r###"9146a7ef6149968c24bb865f92f7fb20830c90a4b9eba03049ce4dc83ca90242"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (158306) panicked at sources/default_trailing_comma.rs:1:13:
assertion failed: false
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"assertion failed: false"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"custom_trailing_comma"###,
            group: r###"append"###,
            source: r###"fn main() { assert!(false, "custom comma",); }
"###,
            source_bytes: 47,
            source_fingerprint: 949848087217664129,
            source_sha256: r###"d25e538cd8a1546dd583dd98539461f03170c0eba1778995e679b8bd6f31cb5b"###,
            native: Native::Abort,
            boundary: r###""###,
            stock_compile_code: 0,
            stock_code: Some(-6),
            stock_stdout: r###""###,
            stock_stderr: r###"
thread 'main' (158342) panicked at sources/custom_trailing_comma.rs:1:13:
custom comma
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
            stock_compile_stderr: r###""###,
            stock_errors: &[],
            message: r###"custom comma"###,
            location: r###"1:13"###,
        },
        Case {
            name: r###"missing_semicolon"###,
            group: r###"append"###,
            source: r###"fn main() { assert!(true) println!("after"); }
"###,
            source_bytes: 47,
            source_fingerprint: 9049479866542366999,
            source_sha256: r###"c581c35021c8ceef46f83f3d0d9e492c3de0590e7467b72c96f8c805abf04b0a"###,
            native: Native::Invalid,
            boundary: r###"semicolon"###,
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: expected one of `.`, `;`, `?`, `}`, or an operator, found `println`
 --> sources/missing_semicolon.rs:1:27
  |
1 | fn main() { assert!(true) println!("after"); }
  |                           ^^^^^^^ expected one of `.`, `;`, `?`, `}`, or an operator

error: aborting due to 1 previous error

"###,
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
            "jr-assertion-{}-{nonce}-{}",
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
            stderr.contains(if !stock && case.name == "deferred_assert_eq" {
                "assertion failed: left == right"
            } else if !stock && case.message.starts_with("assertion failed") {
                "assertion failed"
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
            eprintln!("ASSERTION CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("ASSERTION CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} assertion failures:\n{}",
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
fn frozen_assertion_sources_and_phase_partitions() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 33);
    let mut groups = [0; 2];
    let mut partition = [0; 4];
    let mut stock = [0; 3];
    for case in &all {
        assert_eq!(case.source.len(), case.source_bytes, "{}", case.name);
        assert_eq!(
            fingerprint(case.source),
            case.source_fingerprint,
            "{}",
            case.name
        );
        assert_eq!(case.source_sha256.len(), 64);
        assert!(case.source_sha256.bytes().all(|b| b.is_ascii_hexdigit()));
        assert!(case.source.ends_with('\n'));
        groups[usize::from(case.group == "append")] += 1;
        partition[match case.native {
            Native::Live => 0,
            Native::Abort => 1,
            Native::Invalid => 2,
            Native::Unsupported => 3,
        }] += 1;
        stock[if case.stock_compile_code != 0 {
            2
        } else if case.stock_code == Some(-6) {
            1
        } else {
            0
        }] += 1;
        if case.stock_compile_code != 0 {
            assert!(case.stock_code.is_none());
            assert!(!case.stock_compile_stderr.is_empty());
            for code in case.stock_errors {
                assert!(case.stock_compile_stderr.contains(code));
            }
        }
        if !case.message.is_empty() {
            assert!(case.stock_stderr.contains(case.message));
            assert!(
                case.stock_stderr
                    .contains(&format!(".rs:{}:", case.location))
            );
        }
        match frontend::parse(case.source) {
            Ok(program) => match case.native {
                Native::Live | Native::Abort => {
                    frontend::check(&program).expect("positive all-body check");
                }
                Native::Invalid
                    if case.boundary == "bool_type" || case.boundary == "missing_name" =>
                {
                    diagnostic(
                        case,
                        &frontend::check(&program)
                            .expect_err("invalid condition must reject")
                            .to_string(),
                    );
                }
                _ => panic!("{} crossed frozen parse rejection boundary", case.name),
            },
            Err(error) => {
                assert!(
                    case.native == Native::Unsupported || case.name == "missing_semicolon",
                    "{} rejected before type/name check: {error}",
                    case.name
                );
                diagnostic(case, &error.to_string());
            }
        }
    }
    assert_eq!(groups, [30, 3]);
    assert_eq!(partition, [10, 13, 4, 6]);
    assert_eq!(stock, [14, 14, 5]);
    let joined: String = all.iter().map(|case| case.source).collect();
    assert_eq!(fingerprint(&joined), 6894148404861029812);
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
        "ASSERTION ACTUAL STOCK:\n{}",
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
                        "--crate-name=assertion_oracle",
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
                if case.name == "deferred_nonliteral_message" {
                    assert!(text.contains("format argument must be a string literal"));
                }
            }
        });
    }
    failures.finish();
}
#[test]
fn stock_assertion_exact_sources_and_appended_controls() {
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
            assert!(
                !text.contains("unsupported"),
                "{}: invalid source is not capability error: {text}",
                case.name
            );
            if case.boundary == "bool_type" {
                assert!(
                    text.contains("expected") && text.contains("Bool"),
                    "{}: {text}",
                    case.name
                );
            } else if case.name == "missing_semicolon" {
                assert!(
                    text.contains("expected semicolon or block end"),
                    "{}: {text}",
                    case.name
                );
            } else {
                assert!(text.contains(case.boundary), "{}: {text}", case.name);
            }
        }
        Native::Unsupported => assert!(
            text.contains(case.boundary),
            "{}: expected earlier boundary {:?}: {text}",
            case.name,
            case.boundary
        ),
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
    let program = frontend::parse(case.source).expect("positive frozen source parses");
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
                source_bodies += 1;
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
    assert_eq!(cells, 132);
    failures.check("runtime count", || assert_eq!(runtimes, 46));
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
                                    !report
                                        .node_evaluations
                                        .iter()
                                        .any(|(key, _)| key.kind == 103),
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
    assert_eq!(cells, 132);
    failures.check("graph runtime count", || assert_eq!(runtimes, 46));
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
fn native_assertion_workers1_8_check_build_actual_elf() {
    isolated(
        "native_assertion_workers1_8_check_build_actual_elf",
        "JR_ASSERTION_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_assertion_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_assertion_workers1_8_check_build_all_body_graph",
        "JR_ASSERTION_GRAPH_CHILD",
        graph_all,
    );
}
