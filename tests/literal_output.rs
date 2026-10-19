//! Frozen20 literal-output corpus, not full formatting support.
//! Stock-first receipt: /home/jeremy/.jcode/scratch/literal-output-stock/results.json
//! Receipt SHA256 fc6e2ce07ef6f51abdfa87952c6b6a51ee965eeab1925a59573de897bd0aba4b
//! Native: 16 live, 4 unsupported/invalid. Stock: 16 pass, 4 reject.
//! Retained sources immutable. Author ran stock only. Root owns debug/release gates.
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
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: r###"print_literal"###,
            source: r###"fn main() { print!("out"); }
"###,
            source_bytes: 29,
            source_fingerprint: 1666336487382809297,
            source_sha256: r###"a615706f470d7dc9ce2403532db90fa6cfed461a7bfcbdd67afd4f19ad2434b6"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"out"###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"eprint_literal"###,
            source: r###"fn main() { eprint!("err"); }
"###,
            source_bytes: 30,
            source_fingerprint: 6842966746796566063,
            source_sha256: r###"90def7f19d2dab0a9367e69fa4037ae8a49e58691743a04f9f95459e7f3e0ab8"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"err"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"eprintln_literal"###,
            source: r###"fn main() { eprintln!("err"); }
"###,
            source_bytes: 32,
            source_fingerprint: 17839713914075137625,
            source_sha256: r###"ed2f2d17be92887e534c76828946f1f54d3a7648238e0ceaf126abcf1ff76fef"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"err
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"separate_order"###,
            source: r###"fn main() { print!("a"); eprint!("x"); print!("b"); eprintln!("y"); println!("c"); }
"###,
            source_bytes: 85,
            source_fingerprint: 16297255239212501982,
            source_sha256: r###"004b78a52fa23f1c5af8e4df7e9a4a6195568586477cdf73707d217fde7a604d"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"abc
"###,
            stock_stderr: r###"xy
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"empty_literals"###,
            source: r###"fn main() { print!(""); eprint!(""); eprintln!(""); }
"###,
            source_bytes: 54,
            source_fingerprint: 296893968807823381,
            source_sha256: r###"6333fadc226b3299d6b7358b6acc0f00398fcf97d60bb594fe22159054595462"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"empty_eprintln"###,
            source: r###"fn main() { eprintln!(); }
"###,
            source_bytes: 27,
            source_fingerprint: 8624976187531825104,
            source_sha256: r###"d758f2c6233c8f6ecfd328d2a4c96447fa64c8687ae7007f68a720529782a9f4"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"unicode_escapes"###,
            source: r###"fn main() { print!("λ雪\n\t"); eprint!("é\u{1f980}"); eprintln!("\\\""); }
"###,
            source_bytes: 78,
            source_fingerprint: 15034764709183821853,
            source_sha256: r###"b6528d7655ae4c0e4fdf07b87b111a96599fd9389d286ec3abb18794660b9794"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"λ雪
	"###,
            stock_stderr: r###"é🦀\"
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"escaped_braces"###,
            source: r###"fn main() { print!("{{out}}"); eprint!("{{err}}"); eprintln!("{{}}"); }
"###,
            source_bytes: 72,
            source_fingerprint: 14395928884865052062,
            source_sha256: r###"da76114fc2f560fc47ad4da2bb75fb3e888ff0186aa16e096319823478563f48"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"{out}"###,
            stock_stderr: r###"{err}{}
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"all_three_delimiters"###,
            source: r###"fn main() { print!("a"); print!["b"]; print!{"c"}; eprint!("d"); eprint!["e"]; eprint!{"f"}; eprintln!("g"); eprintln!["h"]; eprintln!{"i"}; }
"###,
            source_bytes: 143,
            source_fingerprint: 14041436360584871930,
            source_sha256: r###"517d459b464a7144e7344cc6e0349d27da9961ee23e98d4bfa671b531bfc0f33"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"abc"###,
            stock_stderr: r###"defg
h
i
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"trailing_comma"###,
            source: r###"fn main() { print!("a",); eprint!["b",]; eprintln!{"c",}; }
"###,
            source_bytes: 60,
            source_fingerprint: 4377718157322313571,
            source_sha256: r###"1d2e887b5785238f044448a90d9694645ed1d6f5dd4e4e36992507b2338c04c3"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###"a"###,
            stock_stderr: r###"bc
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"lexical_shadow_tt"###,
            source: r###"fn main() { macro_rules! print { ($a:tt) => { eprint!("shadow") }; } print!(123); }
"###,
            source_bytes: 84,
            source_fingerprint: 7877021936846050633,
            source_sha256: r###"4220cf5f20c48ab47d79c002d8e2a2ce224c4727a4ba8bb8104dfa0815285967"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"shadow"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"dead_valid"###,
            source: r###"fn main() { if false { print!("dead"); eprintln!("dead"); } eprint!("live"); }
"###,
            source_bytes: 79,
            source_fingerprint: 5074320323346462063,
            source_sha256: r###"33ace03c01e8f39fd8b6afdfc3de7213a441eed6722febb539ae1c7f02c528e5"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"live"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"unused_valid"###,
            source: r###"fn unused() { print!("unused"); eprint!("unused"); } fn main() { eprintln!("live"); }
"###,
            source_bytes: 86,
            source_fingerprint: 1567818374307850430,
            source_sha256: r###"f1ba495599636cc0065a229a8b9b9843e9f2489fb4a9a19acbe1b7b3937a067b"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"live
"###,
            stock_compile_stderr: r###"warning: function `unused` is never used
 --> sources/unused_valid.rs:1:4
  |
1 | fn unused() { print!("unused"); eprint!("unused"); } fn main() { eprintln!("live"); }
  |    ^^^^^^
  |
  = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: 1 warning emitted

"###,
        },
        Case {
            name: r###"dead_invalid"###,
            source: r###"fn main() { if false { print!("{}", 1); } }
"###,
            source_bytes: 44,
            source_fingerprint: 12956414326750447664,
            source_sha256: r###"7fbb79ce92721d460eac1913a4bb2ed7e186ca6af30b5b052db3fa1cad4e052d"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"unused_invalid"###,
            source: r###"fn unused() { eprint!("{}", 1); } fn main() {}
"###,
            source_bytes: 47,
            source_fingerprint: 6610763975645302790,
            source_sha256: r###"b24a0e384cde9676a1b8a0694304cc1676bb82cd3da2f77c9259fc79e3331bdd"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"warning: function `unused` is never used
 --> sources/unused_invalid.rs:1:4
  |
1 | fn unused() { eprint!("{}", 1); } fn main() {}
  |    ^^^^^^
  |
  = note: `#[warn(dead_code)]` (part of `#[warn(unused)]`) on by default

warning: 1 warning emitted

"###,
        },
        Case {
            name: r###"nonliteral"###,
            source: r###"fn main() { print!(123); }
"###,
            source_bytes: 27,
            source_fingerprint: 8541224383322288925,
            source_sha256: r###"536d66de56f9d1d117eb44d3bb475ff4d2485dded2a60da17be2c365c83c9279"###,
            native: Native::Unsupported,
            boundary: r###"print! nonliteral arguments are unsupported"###,
            phase: "parse",
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: format argument must be a string literal
 --> sources/nonliteral.rs:1:20
  |
1 | fn main() { print!(123); }
  |                    ^^^
  |
help: you might be missing a string literal to format with
  |
1 | fn main() { print!("{}", 123); }
  |                    +++++

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"format_arguments"###,
            source: r###"fn main() { eprintln!("literal", 1); }
"###,
            source_bytes: 39,
            source_fingerprint: 15547478099473764768,
            source_sha256: r###"7963f3634c5ce0856c43b4c14c172897f3d99ea5cda86d4448f79261cba988a8"###,
            native: Native::Unsupported,
            boundary: r###"format argument count mismatch"###,
            phase: "parse",
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: argument never used
 --> sources/format_arguments.rs:1:34
  |
1 | fn main() { eprintln!("literal", 1); }
  |                       ---------  ^ argument never used
  |                       |
  |                       formatting specifier missing
  |
help: format specifiers use curly braces, consider adding a format specifier
  |
1 | fn main() { eprintln!("literal{}", 1); }
  |                               ++

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"format_placeholder"###,
            source: r###"fn main() { eprintln!("{}", 1); }
"###,
            source_bytes: 34,
            source_fingerprint: 13385027587059320723,
            source_sha256: r###"0af90f83d611c3f65da515068267eafddfe5eee8476b7a06fad5aabc8c526af9"###,
            native: Native::Live,
            boundary: r###""###,
            phase: "parse",
            stock_compile_code: 0,
            stock_code: Some(0),
            stock_stdout: r###""###,
            stock_stderr: r###"1
"###,
            stock_compile_stderr: r###""###,
        },
        Case {
            name: r###"empty_print"###,
            source: r###"fn main() { print!(); }
"###,
            source_bytes: 24,
            source_fingerprint: 14595855007312711809,
            source_sha256: r###"3a421cd0c09f0257a894ad3bb3d3bc3c90fbc4cc26597b44f0ffebc98c6dacc5"###,
            native: Native::Unsupported,
            boundary: r###"print! nonliteral arguments are unsupported"###,
            phase: "parse",
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: requires at least a format string argument
 --> sources/empty_print.rs:1:13
  |
1 | fn main() { print!(); }
  |             ^^^^^^^^

error: aborting due to 1 previous error

"###,
        },
        Case {
            name: r###"empty_eprint"###,
            source: r###"fn main() { eprint!(); }
"###,
            source_bytes: 25,
            source_fingerprint: 11688620598449039034,
            source_sha256: r###"92b7519db4e4c1aea5402e7e13f7c262a2c0c0c48ecabcaebb1c8b0836fa994d"###,
            native: Native::Unsupported,
            boundary: r###"eprint! nonliteral arguments are unsupported"###,
            phase: "parse",
            stock_compile_code: 1,
            stock_code: None,
            stock_stdout: r###""###,
            stock_stderr: r###""###,
            stock_compile_stderr: r###"error: requires at least a format string argument
 --> sources/empty_eprint.rs:1:13
  |
1 | fn main() { eprint!(); }
  |             ^^^^^^^^^

error: aborting due to 1 previous error

"###,
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
            "jr-literal_output-{}-{nonce}-{}",
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
            eprintln!("LITERAL_OUTPUT CELL FAILURE: {failure}");
            self.0.push(failure);
        } else {
            eprintln!("LITERAL_OUTPUT CELL PASS: {label}");
        }
    }
    fn finish(self) {
        assert!(
            self.0.is_empty(),
            "{} literal_output failures:\n{}",
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
fn frozen_literal_output_sources_and_strict_phases() {
    stock_all();
    let all = cases();
    assert_eq!(all.len(), 20);
    let mut failures = CellFailures::default();
    let mut partition = [0; 2];
    for case in &all {
        partition[match case.native {
            Native::Live => 0,
            Native::Unsupported => 1,
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
            if case.phase == "parse" && matches!(case.native, Native::Unsupported) {
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
    assert_eq!(partition, [16, 4]);
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
        "LITERAL_OUTPUT ACTUAL STOCK:\n{}",
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
                        "--crate-name=literal_output_oracle",
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
fn stock_literal_output_exact_sources() {
    stock_all();
}
fn diagnostic(case: &Case, text: &str) {
    assert!(
        matches!(case.native, Native::Unsupported),
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
fn native_literal_output_workers1_8_check_build_actual_elf() {
    isolated(
        "native_literal_output_workers1_8_check_build_actual_elf",
        "JR_LITERAL_OUTPUT_CLI_CHILD",
        cli_all,
    );
}
#[test]
fn native_literal_output_workers1_8_check_build_all_body_graph() {
    isolated(
        "native_literal_output_workers1_8_check_build_all_body_graph",
        "JR_LITERAL_OUTPUT_GRAPH_CHILD",
        graph_all,
    );
}
