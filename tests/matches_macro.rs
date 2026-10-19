//! Frozen20 bounded matches! cases plus a separate appended local tt shadow.
//! Stock first: /home/jeremy/.jcode/scratch/matches-macro-stock/results.json
//! Receipt SHA256 7a98ef13729f223511d2c0133d68e5f1df35bf692da1297f290cfa42e3ae83d7
//! Actual /usr/bin/rustc, edition2024, panic=abort, no RUSTC_BOOTSTRAP.
//! Stock: 19 pass, 2 reject. Frozen stock bytes and receipts remain unchanged.
//! Current native: 18 live, 1 unsupported, 2 invalid, 0 syntax.
//! Root scalar OR is promoted by the bounded original-source OR milestone.
//! Range and guards retain their separately admitted behavior.
//! Sources frozen before native checks. Root owns native gates.
//! Module shadow stays unsupported; local_shadow is the distinct final append.
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
    // Partition slot retained after `range` became Live; no frozen case uses it now.
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
    stock_stdout: &'static str,
    stock_stderr: &'static str,
    stock_compile_stderr: &'static str,
}
#[rustfmt::skip]
fn cases() -> Vec<Case> {
    vec![
        Case { name: r###"integer"###, source: r###"fn main(){ if matches!(3, 3) { println!("yes"); } if matches!(3, 4) { println!("bad"); } }
"###, source_sha256: r###"f90207b1aec042c212a31c88befb561b65bf2c37df7bd63e9e9dffdc4d4166fa"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"yes
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 91, source_fingerprint: 6768898577358886944, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"boolean"###, source: r###"fn main(){ if matches!(false, false) { println!("bool"); } }
"###, source_sha256: r###"5d04230af0ceae5528997c93994cf068be7f917db2d8d6bfecb89866f28217d3"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"bool
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 61, source_fingerprint: 18265286624080065559, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"character"###, source: r###"fn main(){ if matches!('x', 'x') { println!("char"); } }
"###, source_sha256: r###"91b1bcbac76b43555449dd67711895003afa1b8492e68d89ad74733f5b05be09"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"char
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 57, source_fingerprint: 14947838046542800197, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"negative"###, source: r###"fn main(){ if matches!(-3, -3) { println!("negative"); } }
"###, source_sha256: r###"afc027825c31f8c35a7946d9b99e5c3ecb4a6e01c8c3e37944d2130af643ef30"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"negative
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 59, source_fingerprint: 14223120719545479342, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"brackets"###, source: r###"fn main(){ if matches![7, 7] { println!("bracket"); } }
"###, source_sha256: r###"7ad247e8894dce0e013ebb60d76d087984f502aa7c230855e64f5b8a21af0470"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"bracket
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 56, source_fingerprint: 8178522289842454230, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"braces_trailing"###, source: r###"fn main(){ if matches!{7, 7,} { println!("brace"); } }
"###, source_sha256: r###"fbf2a481626c3df4a5f93b92f201815faebbc6bcc58c4c53722b082ce7c3a628"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"brace
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 55, source_fingerprint: 12173267394793474125, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"wildcard_trailing"###, source: r###"fn main(){ if matches!(9, _,) { println!("wildcard"); } }
"###, source_sha256: r###"0c374cf95d72c0d7b1c063d665ecbf6acad1e5e820d4b6cd12ccef378c0c280b"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"wildcard
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 58, source_fingerprint: 17194922350284729409, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"tuple"###, source: r###"fn main(){ if matches!((1, true), (1, true)) { println!("tuple"); } }
"###, source_sha256: r###"1f02414128156f0c12f1b977c8eec827bc79433441677373d8105c0737ed5e7c"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"tuple
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 70, source_fingerprint: 9197893297597570745, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"enum_unit"###, source: r###"enum E { A, B } fn main(){ if matches!(E::A, E::A) { println!("enum"); } if matches!(E::B, E::A) { println!("bad"); } }
"###, source_sha256: r###"4a118b42d63c5708e40b7783674ae5b86b112566cb964fb9a125a9d56cade055"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"enum
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 120, source_fingerprint: 5637471682614566783, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"enum_payload"###, source: r###"enum E { A(i32), B } fn main(){ if matches!(E::A(2), E::A(2)) { println!("payload"); } if matches!(E::B, E::A(_)) { println!("bad"); } }
"###, source_sha256: r###"a03ec863baca749f079134823afb55a52d5922ab3e6f688a9f33c7189b646a46"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"payload
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 137, source_fingerprint: 2402475430962036001, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"once_only"###, source: r###"fn value()->i32 { println!("once"); 4 } fn main(){ if matches!(value(), 4) { println!("hit"); } if matches!(value(), 5) { println!("bad"); } }
"###, source_sha256: r###"1133001c30bc2e1db1bada902415affe50cb6e4e5ad2178d8ff520b30dd3143d"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"once
hit
once
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 143, source_fingerprint: 2525434169137696008, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"dead_false"###, source: r###"fn value()->i32 { println!("bad"); 4 } fn main(){ if false { if matches!(value(), 4) { println!("bad"); } } println!("live"); }
"###, source_sha256: r###"65b93f55bd5e86fd872de206bf410023dbf8498b9ba729aaea8ed8e42cef1fa4"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"live
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 128, source_fingerprint: 1716112063754188588, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"unused_body"###, source: r###"fn unused(){ if matches!(1, 1) { println!("bad"); } } fn main(){ println!("live"); }
"###, source_sha256: r###"0a15497b1928f82c3541117a5df9fdaf8e7cfae03bdc353adba573595c2773fe"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"live
"###, stock_stderr: r###""###, stock_compile_stderr: r###"warning: function `unused` is never used
 --> sources/unused_body.rs:1:4
  |
1 | fn unused(){ if matches!(1, 1) { println!("bad"); } } fn main(){ println!("live"); }
  |    ^^^^^^
  |
  = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: 1 warning emitted

"###, source_bytes: 85, source_fingerprint: 10660392702407342847, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"bool_result"###, source: r###"fn test()->bool { matches!(1, 1) } fn main(){ let yes:bool=matches!(2, 2); if yes { if test() { println!("typed"); } } }
"###, source_sha256: r###"0caf40ef817819d62e071a57b07d92de2b97df73c2b40580d4486495a481dc0b"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"typed
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 121, source_fingerprint: 17827571369782203577, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"dead_invalid"###, source: r###"fn main(){ if false { let _x:i32=matches!(1, 1); } }
"###, source_sha256: r###"4d3d574cf5a95ae5529f4e0241c8499a2f05579c5516f8fe20e2c8619c6a7424"###, boundary: r###"type"###, phase: r###"check"###, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/dead_invalid.rs:1:34
  |
1 | fn main(){ if false { let _x:i32=matches!(1, 1); } }
  |                                  ^^^^^^^^^^^^^^ expected `i32`, found `bool`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###, source_bytes: 53, source_fingerprint: 6842676511613698000, native: Native::Invalid, stock_compile_code: 1, stock_code: None, },
        Case { name: r###"unused_invalid"###, source: r###"fn unused(){ let _x:i32=matches!(1, 1); } fn main(){}
"###, source_sha256: r###"bcab750bd05e6c739375ccc3ec1fd392df2e71e7cf34747e51a8733f725a5568"###, boundary: r###"type"###, phase: r###"check"###, stock_stdout: r###""###, stock_stderr: r###""###, stock_compile_stderr: r###"error[E0308]: mismatched types
 --> sources/unused_invalid.rs:1:25
  |
1 | fn unused(){ let _x:i32=matches!(1, 1); } fn main(){}
  |                         ^^^^^^^^^^^^^^ expected `i32`, found `bool`

error: aborting due to 1 previous error

For more information about this error, try `rustc --explain E0308`.
"###, source_bytes: 54, source_fingerprint: 881276056421007699, native: Native::Invalid, stock_compile_code: 1, stock_code: None, },
        Case { name: r###"guard"###, source: r###"fn main(){ if matches!(1, x if x == 1) { println!("guard"); } }
"###, source_sha256: r###"5875421359eba37b761e61cddb3b94a904ecce723b5df8d0b06e15950ff17edb"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"guard
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 64, source_fingerprint: 13549839185063572285, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"range"###, source: r###"fn main(){ if matches!(2, 1..=3) { println!("range"); } }
"###, source_sha256: r###"71e1655e7716bc7b55e1e4a6fa148c1f797f44011139d518b51d3d3a89d85b28"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"range
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 58, source_fingerprint: 10514283390403151409, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"or_pattern"###, source: r###"fn main(){ if matches!(2, 1 | 2) { println!("or"); } }
"###, source_sha256: r###"01a3675c88f71c8098daee3b1b0943f4de8d3a6bd765585c5783a397aafad098"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"or
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 55, source_fingerprint: 15071238497542704403, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"module_shadow"###, source: r###"macro_rules! matches { ($($x:tt)*) => { true }; } fn main(){ if matches!(not a pattern) { println!("module"); } }
"###, source_sha256: r###"66fd50c39024204a4063efbc46fa55d250cce590419a7706bdd8d74cb6ffc376"###, boundary: r###"module macros in expression position are unsupported"###, phase: r###"parse"###, stock_stdout: r###"module
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 114, source_fingerprint: 1334715629782715367, native: Native::Unsupported, stock_compile_code: 0, stock_code: Some(0), },
        Case { name: r###"local_shadow"###, source: r###"fn main(){ macro_rules! matches { ($($x:tt)*) => { true }; } if matches!(not a pattern) { println!("local"); } }
"###, source_sha256: r###"f98006c76923f754e82654000e278dd807648d3ad0c46449f0a0f312826127a9"###, boundary: r###""###, phase: r###"check"###, stock_stdout: r###"local
"###, stock_stderr: r###""###, stock_compile_stderr: r###""###, source_bytes: 113, source_fingerprint: 15794649429856441126, native: Native::Live, stock_compile_code: 0, stock_code: Some(0), },
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
            "jr-matches_macro-{}-{nonce}-{}",
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
fn exact_runtime(case: &Case, result: &Captured) {
    ordinary_exit(result, case.stock_code.expect("compiled stock case"));
    assert_eq!(
        result.stdout,
        case.stock_stdout.as_bytes(),
        "{} stdout: {result:?}",
        case.name
    );
    assert_eq!(
        result.stderr,
        case.stock_stderr.as_bytes(),
        "{} stderr: {result:?}",
        case.name
    );
}
fn runtime(case: &Case, path: &Path, _source: &Path, scratch: &Scratch, _stock: bool) {
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(Command::new(path).env("RUST_BACKTRACE", "0"), scratch, 5);
    exact_runtime(case, &result);
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
            eprintln!("MATCHES_MACRO CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("MATCHES_MACRO CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} matches_macro failures:\n{}",
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
fn frozen_matches_macro_sources_and_strict_phases() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 21);
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
            assert!(case.source.ends_with('\n'));
            if case.stock_compile_code != 0 {
                assert!(!case.stock_compile_stderr.is_empty());
            }
            let parsed = frontend::parse(case.source);
            if case.phase == "parse" && matches!(case.native, Native::Unsupported | Native::Syntax)
            {
                diagnostic(
                    case,
                    &parsed.expect_err("frozen parse rejection").to_string(),
                );
            } else {
                let program = parsed.expect("must pass frozen parse phase");
                let checked = frontend::check(&program);
                if matches!(case.native, Native::Live) {
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
    // character and range became Live with SCALAR-PATTERNS.md (sources unchanged).
    assert_eq!(partition, [18, 1, 2, 0]);
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
        "MATCHES_MACRO ACTUAL STOCK:\n{}",
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
                        "--crate-name=matches_macro_oracle",
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
fn stock_matches_macro_exact_sources() {
    stock_all();
}
fn diagnostic(case: &Case, text: &str) {
    assert!(!text.is_empty(), "{}: empty diagnostic", case.name);
    assert!(!text.contains("panicked at"), "{}: {text}", case.name);
    match case.native {
        Native::Live => panic!("{} unexpectedly rejected: {text}", case.name),
        Native::Unsupported => {
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
            assert!(
                text.contains("expected") && text.contains("I32") && text.contains("Bool"),
                "{}: wrong type error: {text}",
                case.name
            );
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
                        let result = capture(&mut command, &scratch, 20);
                        let accepted = matches!(case.native, Native::Live);
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
fn native_matches_macro_workers1_8_check_build_actual_elf() {
    isolated(
        "native_matches_macro_workers1_8_check_build_actual_elf",
        "JR_MATCHES_MACRO_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_matches_macro_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_matches_macro_workers1_8_check_build_all_body_graph",
        "JR_MATCHES_MACRO_GRAPH_CHILD",
        graph_all,
    );
}
