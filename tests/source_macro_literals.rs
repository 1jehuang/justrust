//! Bounded opaque `literal` fragment consumption by the native expression parser.
//! Stock observations precede native expectations. No forwarding user invocation,
//! general string value support, or numeric method support is claimed here.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler::{frontend, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// Independent local baseline, not an assertion about the actual hosted oracle.
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Pre-native stock result JSON SHA256 pins, including all exact source bytes:
// primary13 + discarded suffix3: 3eed03bee3b8ceb0bb9857b9b577b5b20e2289b3f8121e646496558ca23b02dd
// signed minima2 + postfix2: 40846ce2375947c257c2200a2ce354b7a0377132b5df5bae3d8f214e7a69c3a2
// combined builtin println: 3952991ae7a321330ba25097259d1fe78df65e9fc5cf4f35f81c1b39fa53c1db
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeGate {
    Runtime,
    InvalidType,
    InvalidSuffix,
    InvalidLexical,
    UnsupportedStringValue,
    StockOnlyPostfixAtomicity,
}
struct Case {
    name: &'static str,
    source: &'static str,
    stock_stdout: Option<&'static str>,
    stock_diagnostic: Option<&'static str>,
    native: NativeGate,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "integer",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:i32=value!(42);if result==42{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "negative_integer",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:i32=value!(-42);if result == -42{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "spaced_negative_integer",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:i32=value!(- 42);if result == -42{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "float",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:f64=value!(1.5f64);if result==1.5f64{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "negative_float",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:f64=value!(-1.5);if result == -1.5{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "boolean",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:bool=value!(true);if result{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "normal_string_value",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result=value!("ok");if result=="ok"{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "negative_boolean",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result=value!(-true);}"#,
            stock_stdout: None,
            stock_diagnostic: Some("cannot apply unary operator"),
            native: NativeGate::InvalidType,
        },
        Case {
            name: "negative_string",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result=value!(-"ok");}"#,
            stock_stdout: None,
            stock_diagnostic: Some("cannot apply unary operator"),
            // Native `&str` values reject negation in the type checker.
            native: NativeGate::InvalidType,
        },
        Case {
            name: "integer_arbitrary_suffix",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result=value!(1u8x);}"#,
            stock_stdout: None,
            stock_diagnostic: Some("invalid suffix `u8x`"),
            native: NativeGate::InvalidSuffix,
        },
        Case {
            name: "float_arbitrary_suffix",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result=value!(1.5wat);}"#,
            stock_stdout: None,
            stock_diagnostic: Some("invalid suffix `wat`"),
            native: NativeGate::InvalidSuffix,
        },
        Case {
            name: "string_arbitrary_suffix",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result=value!("ok"wat);}"#,
            stock_stdout: None,
            stock_diagnostic: Some("suffixes on string literals are invalid"),
            native: NativeGate::InvalidSuffix,
        },
        Case {
            name: "malformed_radix",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result=value!(0b2);}"#,
            stock_stdout: None,
            stock_diagnostic: Some("invalid digit for a base 2 literal"),
            native: NativeGate::InvalidLexical,
        },
        Case {
            name: "ignored_integer_suffix",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{42};}let result:i32=value!(1u8x);if result==42{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "ignored_float_suffix",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{42};}let result:i32=value!(1.5wat);if result==42{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "ignored_string_suffix",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{42};}let result:i32=value!("ok"wat);if result==42{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "signed_min_i32",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:i32=value!(-2147483648i32);if result == -2147483648i32{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "signed_min_i128",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:i128=value!(-170141183460469231731687303715884105728i128);if result == -170141183460469231731687303715884105728i128{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "negative_opaque_postfix_abs",
            source: r#"fn main(){macro_rules! value{($x:literal)=>{$x};}let result:i32=value!(-1i32).abs();if result==1{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::StockOnlyPostfixAtomicity,
        },
        Case {
            name: "flattened_negative_postfix_abs",
            source: r#"fn main(){let result:i32 = -1i32.abs();if result == -1{println!("ok");}else{println!("bad");}}"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::StockOnlyPostfixAtomicity,
        },
        Case {
            name: "combined_numeric_bool_println_string",
            source: r#"fn forward(n:i32)->i32{n}
fn main(){
 macro_rules! emit { ($n:literal,$flag:literal,$text:literal)=>{if $flag {if forward($n)==-45 {println!($text);}}}; }
 emit!(-45,true,"ok");
}
"#,
            stock_stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
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
            "jr-source-macro-literals-{}-{nonce}-{}",
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
        let mut file = new_file(&self.path(name));
        file.write_all(bytes).unwrap();
        drop(file);
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
    code: i32,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&out))
        .stderr(new_file(&err));
    let mut retries = 0;
    let child = loop {
        match command.spawn() {
            Ok(child) => break child,
            // Pre-spawn Linux ETXTBSY only, never an executed process retry.
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
                fs::metadata(path).unwrap().len() <= 65536,
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
        assert!(fs::metadata(path).unwrap().len() <= 65536);
    }
    Captured {
        code: status.code().expect("signal termination"),
        stdout: fs::read(out).unwrap(),
        stderr: fs::read(err).unwrap(),
    }
}
fn runtime(path: &Path, expected: &str, scratch: &Scratch) {
    assert!(!expected.is_empty(), "runtime oracle cannot be empty");
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(path), scratch, 5);
    assert_eq!(result.code, 0, "runtime: {result:?}");
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn stock_all() {
    let scratch = Scratch::new();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(Command::new(&rustc).arg("-vV"), &scratch, 10);
    assert_eq!(version.code, 0);
    eprintln!(
        "LITERAL SOURCE BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut stock = (0, 0);
    let mut partition = (0, 0, 0, 0, 0, 0);
    for case in cases() {
        match case.native {
            NativeGate::Runtime => partition.0 += 1,
            NativeGate::InvalidType => partition.1 += 1,
            NativeGate::InvalidSuffix => partition.2 += 1,
            NativeGate::InvalidLexical => partition.3 += 1,
            NativeGate::UnsupportedStringValue => partition.4 += 1,
            NativeGate::StockOnlyPostfixAtomicity => partition.5 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=source_macro_literals_oracle",
                    "--color=never",
                    "-Cpanic=abort",
                ])
                .arg(scratch.path("client.rs"))
                .arg("-o")
                .arg(&image),
            &scratch,
            20,
        );
        assert!(result.stdout.is_empty(), "{}:{result:?}", case.name);
        if let Some(expected) = case.stock_stdout {
            stock.0 += 1;
            assert!(case.stock_diagnostic.is_none());
            assert_eq!(result.code, 0, "{}:{result:?}", case.name);
            runtime(&image, expected, &scratch);
        } else {
            stock.1 += 1;
            assert_eq!(result.code, 1, "{}:{result:?}", case.name);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert!(
                diagnostic.contains(case.stock_diagnostic.expect("measured invalid category")),
                "{}:{diagnostic}",
                case.name
            );
            assert!(!image.exists());
        }
    }
    assert_eq!(stock, (15, 6));
    assert_eq!(partition, (13, 2, 3, 1, 0, 2));
}

#[test]
fn stock_literal_consumption_and_phase_controls_edition2024() {
    stock_all();
}

#[test]
fn typed_literal_parser_errors_distinguish_suffix_syntax_from_string_boundaries() {
    let mut counts = (0, 0, 0);
    let mut parsed_type_invalid = 0;
    for case in cases() {
        if case.native == NativeGate::InvalidType {
            let prepared =
                source::prepare_text("literal.rs", case.source, &source::SourceOptions::default())
                    .unwrap_or_else(|error| {
                        panic!("{} must prepare before type rejection:{error}", case.name)
                    });
            frontend::parse_prepared(&prepared).unwrap_or_else(|error| {
                panic!("{} must parse before type rejection:{error}", case.name)
            });
            parsed_type_invalid += 1;
            continue;
        }
        let expected = match case.native {
            NativeGate::InvalidSuffix => {
                counts.0 += 1;
                source::SourceErrorKind::Syntax
            }
            NativeGate::InvalidLexical => {
                counts.1 += 1;
                source::SourceErrorKind::Syntax
            }
            NativeGate::UnsupportedStringValue => {
                counts.2 += 1;
                source::SourceErrorKind::Unsupported
            }
            _ => continue,
        };
        let parsed =
            source::prepare_text("literal.rs", case.source, &source::SourceOptions::default())
                .and_then(|prepared| frontend::parse_prepared(&prepared));
        let error = match parsed {
            Err(error) => error,
            Ok(_) => panic!("{} must reject before publication", case.name),
        };
        assert_eq!(error.kind, expected, "{}:{error}", case.name);
        if case.native == NativeGate::InvalidSuffix {
            // These are lexically valid fragments. The existing expression
            // parser uses Syntax for invalid suffix consumption, even when its
            // legacy message says "unsupported or invalid ... literal suffix".
            assert!(error.message.contains("suffix"), "{}:{error}", case.name);
        }
    }
    assert_eq!(counts, (3, 1, 0));
    assert_eq!(parsed_type_invalid, 2);
}

#[test]
fn native_literal_consumption_workers1_8_check_build_no_compiler_fallback() {
    stock_all();
    let mut gates = 0;
    let mut categories = (0, 0, 0);
    for case in cases() {
        if case.native == NativeGate::StockOnlyPostfixAtomicity {
            // The stock observation proves opaque negative postfix binding,
            // not native abs() method admission or an invalid native source.
            continue;
        }
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                scratch.write("client.rs", case.source.as_bytes());
                let image = scratch.path("native");
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .current_dir(&scratch.0)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"))
                    .arg(scratch.path("client.rs"))
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let result = capture(&mut command, &scratch, 20);
                let diagnostic = String::from_utf8_lossy(&result.stderr);
                assert!(
                    !diagnostic.contains("panicked at"),
                    "{}:{diagnostic}",
                    case.name
                );
                assert_eq!(
                    result.code,
                    i32::from(case.native != NativeGate::Runtime),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                match case.native {
                    NativeGate::Runtime => categories.0 += 1,
                    NativeGate::UnsupportedStringValue => {
                        categories.2 += 1;
                        assert!(result.stdout.is_empty());
                        assert!(
                            diagnostic.contains("unsupported:"),
                            "{} must be explicit Unsupported, not Syntax:{diagnostic}",
                            case.name
                        );
                    }
                    NativeGate::InvalidType
                    | NativeGate::InvalidSuffix
                    | NativeGate::InvalidLexical => {
                        categories.1 += 1;
                        assert!(result.stdout.is_empty());
                        assert!(!diagnostic.is_empty());
                        assert!(
                            !diagnostic.contains("unsupported:"),
                            "{} ordinary invalid must not become Unsupported:{diagnostic}",
                            case.name
                        );
                        if case.native == NativeGate::InvalidSuffix {
                            assert!(diagnostic.contains("suffix"), "{}:{diagnostic}", case.name);
                        }
                    }
                    NativeGate::StockOnlyPostfixAtomicity => unreachable!(),
                }
                if let (false, NativeGate::Runtime, Some(expected)) =
                    (check, case.native, case.stock_stdout)
                {
                    runtime(&image, expected, &scratch);
                } else {
                    for path in ["native", "client", "a.out"] {
                        assert!(
                            !scratch.path(path).exists(),
                            "artifact {path} {}",
                            case.name
                        );
                    }
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 76);
    assert_eq!(categories, (52, 24, 0));
}
