//! Bounded debug_assert!, debug_assert_eq!, debug_assert_ne! acceptance corpus.
//! Stock-first receipt: /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/results.json
//! SHA256 3479b805944fec9cdb2ebf9be806abda922e9b7d69558d7f160730a2d724d61a
//! 53 exact sources, each independently compiled with debug-assertions=yes and no.
//! /usr/bin/rustc 1.98.1, edition2024, panic=abort. Native gates were not run to freeze this corpus.
//! Default panic meaning is required, not stock expression/value rendering.
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
    Abort,
    Invalid,
    Unsupported,
}
struct Frozen {
    compile_code: i32,
    compile_stdout: &'static str,
    compile_stderr: &'static str,
    code: Option<i32>,
    stdout: &'static str,
    stderr: &'static str,
    message: &'static str,
    native_message: &'static str,
    location: &'static str,
}
struct Case {
    name: &'static str,
    source: &'static str,
    source_bytes: usize,
    source_fingerprint: u64,
    source_sha256: &'static str,
    native: Native,
    boundary: &'static str,
    yes: Frozen,
    no: Frozen,
}
impl Case {
    fn stock(&self, enabled: bool) -> &Frozen {
        if enabled { &self.yes } else { &self.no }
    }
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: r###"debug_assert_pass"###,
            source: r###"fn main() { debug_assert!(true); println!("after"); }
"###,
            source_bytes: 54,
            source_fingerprint: 15981575031263492782,
            source_sha256: r###"7fec285a9ba649861295ee02f02e8b76726dab19177b3467048330b9f08a073a"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_fail"###,
            source: r###"fn main() { debug_assert!(false); println!("after"); }
"###,
            source_bytes: 55,
            source_fingerprint: 6402742139819153627,
            source_sha256: r###"86655ed92bac48c34e05b316e760e8670b54c57684b7554fd3df0fa9a7a8a654"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1625149) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_fail.rs:1:13:
assertion failed: false
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion failed: false"###,
                native_message: r###"assertion failed"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_bracket"###,
            source: r###"fn main() { debug_assert![true]; println!("after"); }
"###,
            source_bytes: 54,
            source_fingerprint: 1649447101927666119,
            source_sha256: r###"3ae1862a0efe12b8eba2bc95e3e9fa22afc289edfb7382778bb7b6152a653f75"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_brace"###,
            source: r###"fn main() { debug_assert!{true}; println!("after"); }
"###,
            source_bytes: 54,
            source_fingerprint: 4105005111010760199,
            source_sha256: r###"32fce783cc2e2aa69f036bb473e89c28dcc1dc081cef17056b08349817b76eb1"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_trailing"###,
            source: r###"fn main() { debug_assert!(true,); println!("after"); }
"###,
            source_bytes: 55,
            source_fingerprint: 13854699137619894640,
            source_sha256: r###"4813743ca444d52cd66c0ae0444a7af3f2787dd516eaf7bce9a32a3c92a0d64b"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_literal"###,
            source: r###"fn main() { debug_assert!(false, "literal {{braces}} échec 雪"); println!("after"); }
"###,
            source_bytes: 88,
            source_fingerprint: 12520569371882061197,
            source_sha256: r###"e213951bb9bd57aa8c757a62c9a898c0c5fca88eb3e8ff845e63bb5f27d00264"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1625466) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_literal.rs:1:13:
literal {braces} échec 雪
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"literal {braces} échec 雪"###,
                native_message: r###"literal {braces} échec 雪"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_unit"###,
            source: r###"fn unit(_: ()) { println!("unit"); } fn main() { unit(debug_assert!(true)); }
"###,
            source_bytes: 78,
            source_fingerprint: 8766665203995013176,
            source_sha256: r###"aef3cfcf187d063e74e98cd1692838b3d1ea15874f9480de6a4c7a51a5ed8f1d"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"unit
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"unit
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_format"###,
            source: r###"fn main() { debug_assert!(false, "value {}", 7); }
"###,
            source_bytes: 51,
            source_fingerprint: 18194845124967491999,
            source_sha256: r###"de6d1d3f77831189157eedf77fd3666c869b278673bb558c2543f2561d932dc5"###,
            native: Native::Unsupported,
            boundary: r###"formatting placeholders are unsupported"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1625604) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_format.rs:1:13:
value 7
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"value 7"###,
                native_message: r###"value 7"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_format_pass"###,
            source: r###"fn main() { debug_assert!(true, "value {}", 7); println!("after"); }
"###,
            source_bytes: 69,
            source_fingerprint: 11693586500851936498,
            source_sha256: r###"8d9cc486691bbd2a730253b8302309256132839adfbf84787dba664d6dce71d0"###,
            native: Native::Unsupported,
            boundary: r###"formatting placeholders are unsupported"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_shadow"###,
            source: r###"fn main() { macro_rules! debug_assert { (false) => { println!("shadow") }; } debug_assert!(false); println!("after"); }
"###,
            source_bytes: 120,
            source_fingerprint: 2343281398492867175,
            source_sha256: r###"59b7d51621643d0ad2ec245ce67520f822a750bcd241b340f435097722e0fc05"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_expr_shadow"###,
            source: r###"fn main() { macro_rules! debug_assert { ($e:expr) => { println!("shadow") }; } debug_assert!(false); }
"###,
            source_bytes: 103,
            source_fingerprint: 7534951123322422664,
            source_sha256: r###"d67e0606f096895aa94e9308423dac86429ad07f39e87f07c05d3071d8ac5419"###,
            native: Native::Unsupported,
            boundary: r###"unsupported macro fragment kind"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_pass"###,
            source: r###"fn main() { debug_assert_eq!(1, 1); println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 7818895032182046031,
            source_sha256: r###"d72a6b191fa3ce92315c0451080999decdfd070cae5eaf4b613999944c99f210"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_fail"###,
            source: r###"fn main() { debug_assert_eq!(1, 2); println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 9310351945402435394,
            source_sha256: r###"a119f3ce116188e0fa96b76454f86a7ceec85168b44d71fb8ec0ab27c829c15d"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1625982) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_fail.rs:1:13:
assertion `left == right` failed
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion `left == right` failed"###,
                native_message: r###"assertion failed: left == right"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_bracket"###,
            source: r###"fn main() { debug_assert_eq![1, 1]; println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 1659192999964694326,
            source_sha256: r###"34f46c33e0aa69cdd964cd73967734143f4bec42bfc0b9b212c883d54bce0bfe"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_brace"###,
            source: r###"fn main() { debug_assert_eq!{1, 1}; println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 2218518831280509366,
            source_sha256: r###"200ce2e773bfbe5dea4ecf5d1691eeb4e56dffdf363d90366494a10c75032700"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_trailing"###,
            source: r###"fn main() { debug_assert_eq!(1, 1,); println!("after"); }
"###,
            source_bytes: 58,
            source_fingerprint: 4643109438263744151,
            source_sha256: r###"b2d1da8d5a212701890ea9cef2e47b185cac8749812704215fad3ccd7b2c8cd9"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_literal"###,
            source: r###"fn main() { debug_assert_eq!(1, 2, "literal {{braces}} échec 雪"); println!("after"); }
"###,
            source_bytes: 90,
            source_fingerprint: 13086100433679139366,
            source_sha256: r###"9d7735c63d3ac27768b487e62d60a51057ee71871074092839440804cf56cc6b"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1626245) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_literal.rs:1:13:
assertion `left == right` failed: literal {braces} échec 雪
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion `left == right` failed: literal {braces} échec 雪"###,
                native_message: r###"literal {braces} échec 雪"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_unit"###,
            source: r###"fn unit(_: ()) { println!("unit"); } fn main() { unit(debug_assert_eq!(1, 1)); }
"###,
            source_bytes: 81,
            source_fingerprint: 4839957975522957887,
            source_sha256: r###"4810f72956bc618ef53613a9d80808a41c498d0d02a4d1573f4e82a690afd3e5"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"unit
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"unit
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_format"###,
            source: r###"fn main() { debug_assert_eq!(1, 2, "value {}", 7); }
"###,
            source_bytes: 53,
            source_fingerprint: 6801010153360938218,
            source_sha256: r###"8e9c32e6cc62ec25200df4b67c52aae0e48d92c6976e4d3f96a63780ccb28cb9"###,
            native: Native::Unsupported,
            boundary: r###"formatting placeholders are unsupported"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1626478) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_format.rs:1:13:
assertion `left == right` failed: value 7
  left: 1
 right: 2
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion `left == right` failed: value 7"###,
                native_message: r###"assertion failed: left == right"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_format_pass"###,
            source: r###"fn main() { debug_assert_eq!(1, 1, "value {}", 7); println!("after"); }
"###,
            source_bytes: 72,
            source_fingerprint: 5712093005672724061,
            source_sha256: r###"8fb918984bad6577c205f413b4914761b2975ae1b155259e1b070febfadd9111"###,
            native: Native::Unsupported,
            boundary: r###"formatting placeholders are unsupported"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_shadow"###,
            source: r###"fn main() { macro_rules! debug_assert_eq { (1, 2) => { println!("shadow") }; } debug_assert_eq!(1, 2); println!("after"); }
"###,
            source_bytes: 124,
            source_fingerprint: 2745927670587682775,
            source_sha256: r###"8070f2712816c289ac0366d203b146909be54f29cc8a1e11cd69a75a7683cf12"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_expr_shadow"###,
            source: r###"fn main() { macro_rules! debug_assert_eq { ($e:expr) => { println!("shadow") }; } debug_assert_eq!(false); }
"###,
            source_bytes: 109,
            source_fingerprint: 9309060751089381866,
            source_sha256: r###"fc52d30a454cecf17925d6ff42b7be5d35be558c46ed9aae64d81d72f7b12be5"###,
            native: Native::Unsupported,
            boundary: r###"unsupported macro fragment kind"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_pass"###,
            source: r###"fn main() { debug_assert_ne!(1, 2); println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 12097160077259865977,
            source_sha256: r###"c97f389adb46fc1fb7b6896cdcc26fd149dac927da878562ccf907bc50da7c60"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_fail"###,
            source: r###"fn main() { debug_assert_ne!(1, 1); println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 15613776908527893056,
            source_sha256: r###"84922acdd5fb7c18dc3ec0c9e377ff70d1b770e27fecda40cc4ca41d45d445e1"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1626811) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_fail.rs:1:13:
assertion `left != right` failed
  left: 1
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion `left != right` failed"###,
                native_message: r###"assertion failed: left != right"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_bracket"###,
            source: r###"fn main() { debug_assert_ne![1, 2]; println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 9039439278316301052,
            source_sha256: r###"db6b02af2183d89e60a97506ef99ad9ee3bcc4419feb76901a8bc327a132abab"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_brace"###,
            source: r###"fn main() { debug_assert_ne!{1, 2}; println!("after"); }
"###,
            source_bytes: 57,
            source_fingerprint: 15871545589634134332,
            source_sha256: r###"13aecd3cae3d39c8916d743eb204e9712b1c75c87877512b988c884719d08541"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_trailing"###,
            source: r###"fn main() { debug_assert_ne!(1, 2,); println!("after"); }
"###,
            source_bytes: 58,
            source_fingerprint: 2591880375617129501,
            source_sha256: r###"b62f6d3f9879741c01de0fc40fdb24eb0c2443bf60d898a67f248a3583d98753"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_literal"###,
            source: r###"fn main() { debug_assert_ne!(1, 1, "literal {{braces}} échec 雪"); println!("after"); }
"###,
            source_bytes: 90,
            source_fingerprint: 5380374100174187872,
            source_sha256: r###"ded833e0d7c7ade68cf8e50490f9dd6df7f5d6f5f769f3096636b177caedb643"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1627048) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_literal.rs:1:13:
assertion `left != right` failed: literal {braces} échec 雪
  left: 1
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion `left != right` failed: literal {braces} échec 雪"###,
                native_message: r###"literal {braces} échec 雪"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_unit"###,
            source: r###"fn unit(_: ()) { println!("unit"); } fn main() { unit(debug_assert_ne!(1, 2)); }
"###,
            source_bytes: 81,
            source_fingerprint: 4452896657775761689,
            source_sha256: r###"d3c35d62be3585802d20b50875dc9b3a8b588948516e2a0e0d6e5da337745f27"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"unit
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"unit
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_format"###,
            source: r###"fn main() { debug_assert_ne!(1, 1, "value {}", 7); }
"###,
            source_bytes: 53,
            source_fingerprint: 951729153039244548,
            source_sha256: r###"1596ab9785209415a3940c4d72e3ced6ce8b3f9e3a7351d9f382801bf64b02df"###,
            native: Native::Unsupported,
            boundary: r###"formatting placeholders are unsupported"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###""###,
                stderr: r###"
thread 'main' (1627219) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_format.rs:1:13:
assertion `left != right` failed: value 7
  left: 1
 right: 1
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion `left != right` failed: value 7"###,
                native_message: r###"assertion failed: left != right"###,
                location: r###"1:13"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_format_pass"###,
            source: r###"fn main() { debug_assert_ne!(1, 2, "value {}", 7); println!("after"); }
"###,
            source_bytes: 72,
            source_fingerprint: 5601958501695002775,
            source_sha256: r###"451aa2936c07aa44b4fbefc5ae9cbfd56e4fbaff5bbf2afc6fc73eac2de1bcc2"###,
            native: Native::Unsupported,
            boundary: r###"formatting placeholders are unsupported"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_shadow"###,
            source: r###"fn main() { macro_rules! debug_assert_ne { (1, 1) => { println!("shadow") }; } debug_assert_ne!(1, 1); println!("after"); }
"###,
            source_bytes: 124,
            source_fingerprint: 6783067209017762499,
            source_sha256: r###"de475cc895d1127751cd2a181bbc7f3db045547baaf6de7a95d299d7ff095898"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_expr_shadow"###,
            source: r###"fn main() { macro_rules! debug_assert_ne { ($e:expr) => { println!("shadow") }; } debug_assert_ne!(false); }
"###,
            source_bytes: 109,
            source_fingerprint: 1857713787031594176,
            source_sha256: r###"cafa9fc82661b217007b92d4ac2d3a0a1653a236b8b35bfdabf595a5d5fd3808"###,
            native: Native::Unsupported,
            boundary: r###"unsupported macro fragment kind"###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"shadow
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"condition_once"###,
            source: r###"fn hit(n: &mut i32) -> bool { *n += 1; println!("hit"); true } fn main() { let mut n = 0; debug_assert!(hit(&mut n)); if n == 1 { println!("once"); } else { println!("zero"); } }
"###,
            source_bytes: 179,
            source_fingerprint: 10331546492662369163,
            source_sha256: r###"4f9e792b1a473618b4700b4f708bbb3d7b3204c1d56b5addfed75c7bf0f89c39"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"hit
once
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"zero
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"condition_once_fail"###,
            source: r###"fn hit(n: &mut i32) -> bool { *n += 1; println!("hit"); false } fn main() { let mut n = 0; debug_assert!(hit(&mut n)); println!("after"); }
"###,
            source_bytes: 140,
            source_fingerprint: 9707576037519878500,
            source_sha256: r###"cabbf83664c15d01422b1c87f14cd633c2ba784171ef649c8fcf1b455b4a636a"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###"hit
"###,
                stderr: r###"
thread 'main' (1627567) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/condition_once_fail.rs:1:92:
assertion failed: hit(&mut n)
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"assertion failed: hit(&mut n)"###,
                native_message: r###"assertion failed"###,
                location: r###"1:92"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_operands_once"###,
            source: r###"fn left(n: &mut i32) -> i32 { *n += 1; println!("left"); 1 } fn right(n: &mut i32) -> i32 { *n += 1; println!("right"); 1 } fn main() { let mut n = 0; debug_assert_eq!(left(&mut n), right(&mut n)); if n == 2 { println!("once"); } else { println!("zero"); } }
"###,
            source_bytes: 259,
            source_fingerprint: 13030951682495412356,
            source_sha256: r###"45a797f0336d858e16bd4c0a056fd96a16cf64c47bcecf331b2aa0faee347d99"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"left
right
once
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"zero
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_operands_once"###,
            source: r###"fn left(n: &mut i32) -> i32 { *n += 1; println!("left"); 1 } fn right(n: &mut i32) -> i32 { *n += 1; println!("right"); 2 } fn main() { let mut n = 0; debug_assert_ne!(left(&mut n), right(&mut n)); if n == 2 { println!("once"); } else { println!("zero"); } }
"###,
            source_bytes: 259,
            source_fingerprint: 11507298393990694284,
            source_sha256: r###"6546901bbb3c02522091c8eb21ed554d9dc49d7c193b13f715e2c5af529dba7f"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"left
right
once
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"zero
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"condition_panic"###,
            source: r###"fn condition() -> bool { println!("condition"); panic!("condition panic") } fn main() { debug_assert!(condition()); println!("after"); }
"###,
            source_bytes: 137,
            source_fingerprint: 7657229462533303091,
            source_sha256: r###"ef709f4dd1c8771eef3906a94acacf9615c68eab8d1aa849eba3e167d5193eeb"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###"condition
"###,
                stderr: r###"
thread 'main' (1627764) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/condition_panic.rs:1:49:
condition panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"condition panic"###,
                native_message: r###"condition panic"###,
                location: r###"1:49"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"short_circuit"###,
            source: r###"fn rhs() -> bool { println!("wrong"); false } fn main() { debug_assert!(true || rhs()); println!("after"); }
"###,
            source_bytes: 109,
            source_fingerprint: 2620399426055675503,
            source_sha256: r###"e9c17ba918a25d0fc44b6c4b1defac9e1aa2809eefce10e66006b39eb1753768"###,
            native: Native::Live,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"invalid_bool"###,
            source: r###"fn main() { debug_assert!(7); }
"###,
            source_bytes: 32,
            source_fingerprint: 11297505399890854435,
            source_sha256: r###"f691971bd0db9a6cefdfec28fb11b1905336bddf7f0c3efe6c760af8d98ef98e"###,
            native: Native::Invalid,
            boundary: r###"Bool"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/invalid_bool.rs:1:13
  |
1 | fn main() { debug_assert!(7); }
  |             ^^^^^^^^^^^^^^^^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/invalid_bool.rs:1:13
  |
1 | fn main() { debug_assert!(7); }
  |             ^^^^^^^^^^^^^^^^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"invalid_bool_dead"###,
            source: r###"fn main() { if false { debug_assert!(7); } }
"###,
            source_bytes: 45,
            source_fingerprint: 4695508744725376709,
            source_sha256: r###"2e60541dfb9aa9ee6d2f94066eb23fcd375d71baded379b7057824d983650177"###,
            native: Native::Invalid,
            boundary: r###"Bool"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/invalid_bool_dead.rs:1:24
  |
1 | fn main() { if false { debug_assert!(7); } }
  |                        ^^^^^^^^^^^^^^^^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/invalid_bool_dead.rs:1:24
  |
1 | fn main() { if false { debug_assert!(7); } }
  |                        ^^^^^^^^^^^^^^^^ expected `bool`, found integer

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"invalid_name"###,
            source: r###"fn main() { debug_assert!(missing); }
"###,
            source_bytes: 38,
            source_fingerprint: 5047083435454127032,
            source_sha256: r###"f904bc859bad25d027ec970a0a1ec87623b093a08e750454b5bf4ea553249005"###,
            native: Native::Invalid,
            boundary: r###"missing"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0425]: cannot find value `missing` in this scope
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/invalid_name.rs:1:27
  |
1 | fn main() { debug_assert!(missing); }
  |                           ^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0425]: cannot find value `missing` in this scope
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/invalid_name.rs:1:27
  |
1 | fn main() { debug_assert!(missing); }
  |                           ^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_invalid_type"###,
            source: r###"fn main() { debug_assert_eq!(1, true); }
"###,
            source_bytes: 41,
            source_fingerprint: 14479513780467078890,
            source_sha256: r###"a4bb9b217271df3f9eba500a5ce99ffe3e642dd681d8a8a92c1dad534ac564c8"###,
            native: Native::Invalid,
            boundary: r###"equality operands differ"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_eq!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ expected integer, found `bool`

error[E0277]: can't compare `{integer}` with `bool`
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_eq!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ no implementation for `{integer} == bool`
  |
  = help: the trait `PartialEq<bool>` is not implemented for `{integer}`
  = help: the following other types implement trait `PartialEq<Rhs>`:
            f128
            f16
            f32
            f64
            i128
            i16
            i32
            i64
          and 8 others

error: aborting due to 2 previous errors

Some errors have detailed explanations: E0277, E0308.
For more information about an error, try `rustc --explain E0277`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_eq!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ expected integer, found `bool`

error[E0277]: can't compare `{integer}` with `bool`
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_eq!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ no implementation for `{integer} == bool`
  |
  = help: the trait `PartialEq<bool>` is not implemented for `{integer}`
  = help: the following other types implement trait `PartialEq<Rhs>`:
            f128
            f16
            f32
            f64
            i128
            i16
            i32
            i64
          and 8 others

error: aborting due to 2 previous errors

Some errors have detailed explanations: E0277, E0308.
For more information about an error, try `rustc --explain E0277`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_invalid_name"###,
            source: r###"fn main() { debug_assert_eq!(1, missing); }
"###,
            source_bytes: 44,
            source_fingerprint: 12101684906246172744,
            source_sha256: r###"f431348d72b7d6d27c4ebe593216ded7be1f001fe084864b1a1e66fb0cf1a996"###,
            native: Native::Invalid,
            boundary: r###"missing"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0425]: cannot find value `missing` in this scope
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_invalid_name.rs:1:33
  |
1 | fn main() { debug_assert_eq!(1, missing); }
  |                                 ^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0425]: cannot find value `missing` in this scope
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_invalid_name.rs:1:33
  |
1 | fn main() { debug_assert_eq!(1, missing); }
  |                                 ^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_nonliteral"###,
            source: r###"fn main() { let message = "custom"; debug_assert_eq!(1, 2, message); }
"###,
            source_bytes: 71,
            source_fingerprint: 7869453377562554686,
            source_sha256: r###"565419b04b7df7562ffe530b0d868e0623721a68a87a9f087b9e56a05160400d"###,
            native: Native::Unsupported,
            boundary: r###"debug_assert_eq! nonliteral messages are unsupported"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error: format argument must be a string literal
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_nonliteral.rs:1:60
  |
1 | fn main() { let message = "custom"; debug_assert_eq!(1, 2, message); }
  |                                                            ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; debug_assert_eq!(1, 2, "{}", message); }
  |                                                            +++++

error: aborting due to 1 previous error

"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error: format argument must be a string literal
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_nonliteral.rs:1:60
  |
1 | fn main() { let message = "custom"; debug_assert_eq!(1, 2, message); }
  |                                                            ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; debug_assert_eq!(1, 2, "{}", message); }
  |                                                            +++++

error: aborting due to 1 previous error

"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_invalid_type"###,
            source: r###"fn main() { debug_assert_ne!(1, true); }
"###,
            source_bytes: 41,
            source_fingerprint: 12322611326806409161,
            source_sha256: r###"5d75cabcec9697b943dc2fe26434a2381cda95216dcfe08f2042978f3cbd99af"###,
            native: Native::Invalid,
            boundary: r###"equality operands differ"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_ne!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ expected integer, found `bool`

error[E0277]: can't compare `{integer}` with `bool`
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_ne!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ no implementation for `{integer} == bool`
  |
  = help: the trait `PartialEq<bool>` is not implemented for `{integer}`
  = help: the following other types implement trait `PartialEq<Rhs>`:
            f128
            f16
            f32
            f64
            i128
            i16
            i32
            i64
          and 8 others

error: aborting due to 2 previous errors

Some errors have detailed explanations: E0277, E0308.
For more information about an error, try `rustc --explain E0277`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0308]: mismatched types
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_ne!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ expected integer, found `bool`

error[E0277]: can't compare `{integer}` with `bool`
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_invalid_type.rs:1:13
  |
1 | fn main() { debug_assert_ne!(1, true); }
  |             ^^^^^^^^^^^^^^^^^^^^^^^^^ no implementation for `{integer} == bool`
  |
  = help: the trait `PartialEq<bool>` is not implemented for `{integer}`
  = help: the following other types implement trait `PartialEq<Rhs>`:
            f128
            f16
            f32
            f64
            i128
            i16
            i32
            i64
          and 8 others

error: aborting due to 2 previous errors

Some errors have detailed explanations: E0277, E0308.
For more information about an error, try `rustc --explain E0277`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_invalid_name"###,
            source: r###"fn main() { debug_assert_ne!(1, missing); }
"###,
            source_bytes: 44,
            source_fingerprint: 5084567471991657921,
            source_sha256: r###"187ce09d8ab5616b94dd899022798e86588eb26399f26aef71c89f915d0649f4"###,
            native: Native::Invalid,
            boundary: r###"missing"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0425]: cannot find value `missing` in this scope
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_invalid_name.rs:1:33
  |
1 | fn main() { debug_assert_ne!(1, missing); }
  |                                 ^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error[E0425]: cannot find value `missing` in this scope
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_invalid_name.rs:1:33
  |
1 | fn main() { debug_assert_ne!(1, missing); }
  |                                 ^^^^^^^ not found in this scope

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0425`.
"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_nonliteral"###,
            source: r###"fn main() { let message = "custom"; debug_assert_ne!(1, 2, message); }
"###,
            source_bytes: 71,
            source_fingerprint: 13036606243337873201,
            source_sha256: r###"1afa0cb7f3f2f1d6c5ebdf1d4cc30b991bbd0986872084cbb82623323bedb133"###,
            native: Native::Unsupported,
            boundary: r###"debug_assert_ne! nonliteral messages are unsupported"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error: format argument must be a string literal
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_nonliteral.rs:1:60
  |
1 | fn main() { let message = "custom"; debug_assert_ne!(1, 2, message); }
  |                                                            ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; debug_assert_ne!(1, 2, "{}", message); }
  |                                                            +++++

error: aborting due to 1 previous error

"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error: format argument must be a string literal
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_nonliteral.rs:1:60
  |
1 | fn main() { let message = "custom"; debug_assert_ne!(1, 2, message); }
  |                                                            ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; debug_assert_ne!(1, 2, "{}", message); }
  |                                                            +++++

error: aborting due to 1 previous error

"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_nonliteral"###,
            source: r###"fn main() { let message = "custom"; debug_assert!(false, message); }
"###,
            source_bytes: 69,
            source_fingerprint: 14728022449484480847,
            source_sha256: r###"4466d84f25a578bb78d04fdc26c87056be758b7e6edb83e485d4139ef19fa208"###,
            native: Native::Unsupported,
            boundary: r###"debug_assert! nonliteral messages are unsupported"###,
            yes: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error: format argument must be a string literal
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_nonliteral.rs:1:58
  |
1 | fn main() { let message = "custom"; debug_assert!(false, message); }
  |                                                          ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; debug_assert!(false, "{}", message); }
  |                                                          +++++

error: aborting due to 1 previous error

"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
            no: Frozen {
                compile_code: 1,
                compile_stdout: r###""###,
                compile_stderr: r###"error: format argument must be a string literal
 --> /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_nonliteral.rs:1:58
  |
1 | fn main() { let message = "custom"; debug_assert!(false, message); }
  |                                                          ^^^^^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { let message = "custom"; debug_assert!(false, "{}", message); }
  |                                                          +++++

error: aborting due to 1 previous error

"###,
                code: None,
                stdout: r###""###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_left_panic"###,
            source: r###"fn left() -> i32 { println!("left"); panic!("left panic") } fn right() -> i32 { println!("wrong"); 1 } fn main() { debug_assert_eq!(left(), right()); println!("after"); }
"###,
            source_bytes: 171,
            source_fingerprint: 2132366220266256229,
            source_sha256: r###"1391ecb770e419aa2468fff60c1101df1cf822cef37e7fb495008f94977f5487"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###"left
"###,
                stderr: r###"
thread 'main' (1628024) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_left_panic.rs:1:38:
left panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"left panic"###,
                native_message: r###"left panic"###,
                location: r###"1:38"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_eq_right_panic"###,
            source: r###"fn left() -> i32 { println!("left"); 1 } fn right() -> i32 { println!("right"); panic!("right panic") } fn main() { debug_assert_eq!(left(), right()); println!("after"); }
"###,
            source_bytes: 172,
            source_fingerprint: 15354067423045011883,
            source_sha256: r###"b08eed379ab85faa5db144b57bbca87bb8c58bde3be96db6ae1bec70e2e05ad0"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###"left
right
"###,
                stderr: r###"
thread 'main' (1628099) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_eq_right_panic.rs:1:81:
right panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"right panic"###,
                native_message: r###"right panic"###,
                location: r###"1:81"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_left_panic"###,
            source: r###"fn left() -> i32 { println!("left"); panic!("left panic") } fn right() -> i32 { println!("wrong"); 1 } fn main() { debug_assert_ne!(left(), right()); println!("after"); }
"###,
            source_bytes: 171,
            source_fingerprint: 9711223589659302464,
            source_sha256: r###"5756c024f8e06f66436d219083ebedb80cb2800b77c00c97f488db0927ca39ec"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###"left
"###,
                stderr: r###"
thread 'main' (1628165) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_left_panic.rs:1:38:
left panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"left panic"###,
                native_message: r###"left panic"###,
                location: r###"1:38"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
        },
        Case {
            name: r###"debug_assert_ne_right_panic"###,
            source: r###"fn left() -> i32 { println!("left"); 1 } fn right() -> i32 { println!("right"); panic!("right panic") } fn main() { debug_assert_ne!(left(), right()); println!("after"); }
"###,
            source_bytes: 172,
            source_fingerprint: 10915027683385630058,
            source_sha256: r###"299f34a5555e83481fba3347a6a7be32b61a068241fc12c306d36b943bdfd27e"###,
            native: Native::Abort,
            boundary: r###""###,
            yes: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(-6),
                stdout: r###"left
right
"###,
                stderr: r###"
thread 'main' (1628293) panicked at /home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/debug_assert_ne_right_panic.rs:1:81:
right panic
note: run with `RUST_BACKTRACE=1` environment variable to display a backtrace
"###,
                message: r###"right panic"###,
                native_message: r###"right panic"###,
                location: r###"1:81"###,
            },
            no: Frozen {
                compile_code: 0,
                compile_stdout: r###""###,
                compile_stderr: r###""###,
                code: Some(0),
                stdout: r###"after
"###,
                stderr: r###""###,
                message: r###""###,
                native_message: r###""###,
                location: r###""###,
            },
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
            "jr-debug_assertion-{}-{nonce}-{}",
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
            eprintln!("DEBUG_ASSERTION CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("DEBUG_ASSERTION CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} debug_assertion failures:\n{}",
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
fn stock_all() {
    static STOCK: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    STOCK.get_or_init(|| {
        let scratch = Scratch::new();
        let version = capture(
            Command::new(stock_rustc())
                .env_remove("RUSTC_BOOTSTRAP")
                .arg("-vV"),
            &scratch,
            10,
        );
        ordinary_exit(&version, 0);
        eprintln!(
            "DEBUG_ASSERTION ACTUAL STOCK:\n{}",
            String::from_utf8_lossy(&version.stdout)
        );
        let mut failures = CellFailures::default();
        for case in cases() {
            for enabled in [true, false] {
                failures.check(format!("stock {} debug={enabled}", case.name), || {
                    let scratch = Scratch::new();
                    scratch.write("source.rs", case.source.as_bytes());
                    let source = scratch.path("source.rs");
                    let image = scratch.path("stock");
                    let frozen = case.stock(enabled);
                    let result = capture(
                        Command::new(stock_rustc())
                            .env_remove("RUSTC_BOOTSTRAP")
                            .env("RUST_BACKTRACE", "0")
                            .env("LC_ALL", "C")
                            .args([
                                "--edition=2024",
                                "--crate-name=debug_assertion_oracle",
                                "--color=never",
                                "-Cpanic=abort",
                            ])
                            .arg(if enabled {
                                "-Cdebug-assertions=yes"
                            } else {
                                "-Cdebug-assertions=no"
                            })
                            .arg(&source)
                            .arg("-o")
                            .arg(&image),
                        &scratch,
                        20,
                    );
                    ordinary_exit(&result, frozen.compile_code);
                    assert_eq!(result.stdout, frozen.compile_stdout.as_bytes());
                    // Paths differ per invocation. The complete original bytes are frozen
                    // below and in the receipt, while replay compares stable diagnostics.
                    let text = String::from_utf8_lossy(&result.stderr);
                    let original = format!(
                        "/home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/{}.rs",
                        case.name
                    );
                    let expected = frozen
                        .compile_stderr
                        .replace(&original, &source.display().to_string());
                    assert_eq!(text, expected, "{}: exact stock compile stderr", case.name);
                    let codes = error_codes(&text);
                    assert_eq!(codes, error_codes(frozen.compile_stderr), "{}", case.name);
                    if frozen.compile_code == 0 {
                        assert!(text.is_empty(), "unexpected warning: {text}");
                        runtime(&case, enabled, &image, &source, &scratch, true);
                    } else {
                        assert!(!image.exists());
                        assert!(!text.is_empty());
                        if case.name.ends_with("nonliteral") {
                            assert!(text.contains("format argument must be a string literal"));
                        }
                    }
                });
            }
        }
        failures.finish();
    });
}
fn error_codes(text: &str) -> Vec<&str> {
    text.lines()
        .filter_map(|line| {
            line.strip_prefix("error[")?
                .split_once(']')
                .map(|part| part.0)
        })
        .collect()
}
fn runtime(case: &Case, enabled: bool, path: &Path, source: &Path, scratch: &Scratch, stock: bool) {
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(Command::new(path).env("RUST_BACKTRACE", "0"), scratch, 5);
    let frozen = case.stock(enabled);
    assert_eq!(
        result.stdout,
        frozen.stdout.as_bytes(),
        "{} debug={enabled}: {result:?}",
        case.name
    );
    if frozen.code == Some(-6) {
        assert_eq!(
            result.status.signal(),
            Some(6),
            "{}: SIGABRT required: {result:?}",
            case.name
        );
        assert_eq!(result.status.code(), None);
        let text = String::from_utf8_lossy(&result.stderr);
        if stock {
            let original = format!(
                "/home/jeremy/.jcode/scratch/debug-assert-stock-20261018/sources/{}.rs",
                case.name
            );
            let expected = frozen
                .stderr
                .replace(&original, &source.display().to_string());
            assert_eq!(
                without_pid(&text),
                without_pid(&expected),
                "{}: exact stock runtime stderr",
                case.name
            );
        }
        let message = if stock {
            frozen.message
        } else {
            frozen.native_message
        };
        assert!(
            text.contains(message),
            "{} missing {message:?}: {result:?}",
            case.name
        );
        let location = format!("{}:{}:", source.display(), frozen.location);
        assert!(
            text.contains(&location),
            "{} missing {location:?}: {result:?}",
            case.name
        );
        assert!(!text.lines().any(|line| line == "after"));
    } else {
        ordinary_exit(
            &result,
            frozen.code.expect("accepted source has runtime status"),
        );
        assert!(result.stderr.is_empty(), "{}: {result:?}", case.name);
    }
}
fn without_pid(text: &str) -> String {
    let Some((prefix, rest)) = text.split_once("thread 'main' (") else {
        return text.to_owned();
    };
    let Some((pid, suffix)) = rest.split_once(") panicked at ") else {
        return text.to_owned();
    };
    assert!(!pid.is_empty() && pid.bytes().all(|byte| byte.is_ascii_digit()));
    format!("{prefix}thread 'main' (<pid>) panicked at {suffix}")
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
fn configured(enabled: bool) -> compiler::source::SourceOptions {
    let mut options = compiler::source::SourceOptions::default();
    options.cfg.set_debug_assertions(enabled);
    options
}
#[test]
fn stock_debug_assertions_exact_sources_both_profiles() {
    stock_all();
}
#[test]
fn frozen_debug_assertions_sources_and_profile_independent_rejections() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 53);
    let mut partition = [0; 4];
    for case in &all {
        partition[match case.native {
            Native::Live => 0,
            Native::Abort => 1,
            Native::Invalid => 2,
            Native::Unsupported => 3,
        }] += 1;
    }
    assert_eq!(partition, [22, 12, 7, 12]);
    for enabled in [true, false] {
        assert_eq!(
            all.iter()
                .filter(|case| case.stock(enabled).compile_code == 0)
                .count(),
            43
        );
        assert_eq!(
            all.iter()
                .filter(|case| case.stock(enabled).code == Some(-6))
                .count(),
            if enabled { 15 } else { 0 }
        );
    }
    let mut failures = CellFailures::default();
    for case in &all {
        failures.check(format!("frozen {}", case.name), || {
            assert_eq!(case.source.len(), case.source_bytes);
            assert_eq!(fingerprint(case.source), case.source_fingerprint);
            assert_eq!(case.source_sha256.len(), 64);
            assert!(case.source.ends_with('\n'));
            for enabled in [true, false] {
                let frozen = case.stock(enabled);
                if frozen.compile_code != 0 {
                    assert!(!frozen.compile_stderr.is_empty());
                }
                if frozen.code == Some(-6) {
                    assert!(enabled, "disabled builtin unexpectedly aborts");
                    assert!(frozen.stderr.contains(frozen.message));
                    assert!(frozen.stderr.contains(&format!(".rs:{}:", frozen.location)));
                }
            }
            assert_eq!(
                case.stock(true).compile_code,
                case.stock(false).compile_code
            );
        });
    }
    failures.finish();
}
fn successful_graph(case: &Case, enabled: bool, compiled: &compiler::Compilation, check: bool) {
    let program = if enabled {
        native_prelude::parse(case.source)
    } else {
        native_prelude::parse_configured(case.source, &configured(enabled))
    }
    .expect("configured source and native library parse");
    frontend::check(&program).expect("all bodies checked even with debug assertions disabled");
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
            "{}:{kind}",
            case.name
        );
        assert!(nodes.iter().all(|(_, count)| *count == 1));
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
fn native_all(graph: bool) {
    let mut failures = CellFailures::default();
    let mut cells = 0;
    let mut runtimes = 0;
    for case in cases() {
        for enabled in [true, false] {
            for workers in [1, 8] {
                for check in [true, false] {
                    failures.check(format!("native {} graph={graph} debug={enabled} workers={workers} check={check}", case.name), || {
                        let scratch = Scratch::new();
                        scratch.write("source.rs", case.source.as_bytes());
                        let source = scratch.path("source.rs");
                        let image = scratch.path("native");
                        let accepted = matches!(case.native, Native::Live | Native::Abort);
                        if graph {
                            match compiler::compile_file_observed_with_options(source.clone(), workers, check, configured(enabled)) {
                                Ok(compiled) => {
                                    assert!(accepted, "{} unexpectedly accepted", case.name);
                                    successful_graph(&case, enabled, &compiled, check);
                                    if !check {
                                        scratch.write("native", compiled.executable.as_ref().unwrap());
                                        fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
                                    }
                                }
                                Err(failure) => {
                                    diagnostic(&case, &failure.to_string());
                                    let report = failure.report.expect("rejection retains graph");
                                    assert!(report.capture_complete);
                                    assert_eq!(report.events_dropped, 0);
                                    assert!(report.failures > 0);
                                    assert!(!report.published_kind(105), "rejected source emitted object");
                                }
                            }
                        } else {
                            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                            no_compiler_tools(&mut command, &scratch);
                            command.arg(&source).args(["--workers", &workers.to_string(), "--debug-assertions", if enabled { "yes" } else { "no" }]);
                            if check { command.arg("--check"); } else { command.arg("-o").arg(&image); }
                            let result = capture(&mut command, &scratch, 20);
                            ordinary_exit(&result, i32::from(!accepted));
                            if !accepted {
                                assert!(result.stdout.is_empty());
                                diagnostic(&case, &String::from_utf8_lossy(&result.stderr));
                            }
                        }
                        if accepted && !check {
                            runtime(&case, enabled, &image, &source, &scratch, false);
                            runtimes += 1;
                        } else {
                            for name in ["native", "source", "a.out"] { assert!(!scratch.path(name).exists(), "unexpected artifact {name}"); }
                        }
                    });
                    cells += 1;
                }
            }
        }
    }
    assert_eq!(cells, cases().len() * 8);
    failures.check("all requested runtimes executed", || {
        assert_eq!(
            runtimes,
            cases()
                .iter()
                .filter(|case| matches!(case.native, Native::Live | Native::Abort))
                .count()
                * 4
        )
    });
    failures.finish();
}
fn cli_all() {
    native_all(false);
}
fn graph_all() {
    native_all(true);
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
    let result = capture_bounded(&mut command, &scratch, 480, 16 * 1024 * 1024);
    eprint!("{}", String::from_utf8_lossy(&result.stdout));
    eprint!("{}", String::from_utf8_lossy(&result.stderr));
    ordinary_exit(&result, 0);
}
#[test]
fn native_debug_assertions_both_profiles_workers1_8_check_build_actual_elf() {
    isolated(
        "native_debug_assertions_both_profiles_workers1_8_check_build_actual_elf",
        "JR_DEBUG_ASSERT_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_debug_assertions_both_profiles_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_debug_assertions_both_profiles_workers1_8_check_build_all_body_graph",
        "JR_DEBUG_ASSERT_GRAPH_CHILD",
        graph_all,
    );
}
