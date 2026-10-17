//! Bounded function-local source macro acceptance, not general Rust macro support.
//! The frozen stock corpus was measured before native source admission existed.
//! Native tests are intentionally active, never ignored or preexpanded substitutes.
//! Recursive expansion and nested function-item helpers remain explicit boundaries.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

// This local baseline is independent of the actual toolchain printed on every run.
// Initial stock-only results JSON SHA256:
// 7ed2f4e8e5874f2141f643368626859464ceff6f8a68f8efb9850879c5f02b7a
// Two additional binder controls were measured stock-first separately, JSON SHA256:
// 2b79c667ab124d8b7f803d3a297e48e306db80d37e2bd64fafe81a3fdc242fd9
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum NativeGate {
    Runtime,
    Invalid,
    // These are stock-valid sources, not invalid conformance successes. Their
    // future native policy is explicit Unsupported until the slice is widened.
    StockValidNativeUnsupportedRecursion,
    StockValidNativeUnsupportedBlockItems,
    // Stock recursion-limit rejection does not prove native semantic rejection.
    StockInvalidNativeUnsupportedRecursion,
    StockOnlyLexicalControl,
}
struct Case {
    name: &'static str,
    source: &'static str,
    stdout: Option<&'static str>,
    stock_diagnostic: Option<&'static str>,
    native: NativeGate,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "exact_local_shadow",
            source: r#"fn helper() -> i32 { 2 }
fn main() {
    let x: i32 = 40;
    macro_rules! add {
        ($arg:ident) => { x + helper() + $arg };
    }
    {
        let x: i32 = 1;
        let y: i32 = 3;
        let result: i32 = add!(y);
        if result == 45 {
            println!("ok");
        } else {
            println!("bad");
        }
    }
}
"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "captured_same_spelling",
            source: r#"fn main(){let x:i32=40;macro_rules! add{($arg:ident)=>{x+$arg};}{let x:i32=3;let result:i32=add!(x);if result==43{println!("ok");}else{println!("bad");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "tt_capture_expression",
            source: r#"fn main(){let x:i32=40;macro_rules! add{($($arg:tt)*)=>{x+($($arg)*)};}{let x:i32=1;let y:i32=3;let result:i32=add!(y+2);if result==45{println!("ok");}else{println!("bad");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "definition_before_binding",
            source: r#"fn main(){macro_rules! read{()=>{x};}let x:i32=40;let result:i32=read!();}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find value `x`"),
            native: NativeGate::Invalid,
        },
        Case {
            name: "captured_unknown",
            source: r#"fn main(){let x:i32=40;macro_rules! add{($arg:ident)=>{x+$arg};}let result:i32=add!(unknown);}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find value `unknown`"),
            native: NativeGate::Invalid,
        },
        Case {
            name: "invocation_before_definition",
            source: r#"fn main(){let result:i32=read!();macro_rules! read{()=>{40};}}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `read`"),
            native: NativeGate::Invalid,
        },
        Case {
            name: "no_matching_arm",
            source: r#"fn main(){macro_rules! read{($arg:ident)=>{$arg};}let result:i32=read!(1);}"#,
            stdout: None,
            stock_diagnostic: Some("no rules expected"),
            native: NativeGate::Invalid,
        },
        Case {
            name: "recursive_unbounded",
            source: r#"fn main(){macro_rules! again{()=>{again!()};}let result:i32=again!();}"#,
            stdout: None,
            stock_diagnostic: Some("recursion limit reached"),
            native: NativeGate::StockInvalidNativeUnsupportedRecursion,
        },
        Case {
            name: "recursive_terminating",
            source: r#"fn main(){macro_rules! step{()=>{step!(done)};(done)=>{45};}let result:i32=step!();if result==45{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::StockValidNativeUnsupportedRecursion,
        },
        Case {
            name: "helper_invocation_scope",
            source: r#"fn helper()->i32{2}fn main(){let x:i32=40;macro_rules! add{($arg:ident)=>{x+helper()+$arg};}{fn helper()->i32{5}let y:i32=3;let result:i32=add!(y);if result==48{println!("ok");}else{println!("bad");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::StockValidNativeUnsupportedBlockItems,
        },
        Case {
            name: "helper_only_invocation_scope",
            source: r#"fn main(){let x:i32=40;macro_rules! add{($arg:ident)=>{x+helper()+$arg};}{fn helper()->i32{2}let y:i32=3;let result:i32=add!(y);if result==45{println!("ok");}else{println!("bad");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::StockValidNativeUnsupportedBlockItems,
        },
        Case {
            name: "helper_unknown",
            source: r#"fn main(){let x:i32=40;macro_rules! add{($arg:ident)=>{x+helper()+$arg};}let y:i32=3;let result:i32=add!(y);}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find function `helper`"),
            native: NativeGate::Invalid,
        },
        Case {
            name: "macro_scope_escape",
            source: r#"fn main(){{macro_rules! read{()=>{40};}}let result:i32=read!();}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `read`"),
            native: NativeGate::Invalid,
        },
        Case {
            name: "false_cfg_no_registration",
            source: r#"#[cfg(any())]fn excluded(){macro_rules! read{()=>{40};}}fn main(){let result:i32=read!();}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `read`"),
            native: NativeGate::Invalid,
        },
        Case {
            name: "inactive_lexical_error",
            source: r#"#[cfg(any())]fn excluded(){macro_rules! read{()=>{`};}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("unknown start of token"),
            native: NativeGate::StockOnlyLexicalControl,
        },
        Case {
            name: "generated_binder_does_not_capture_argument",
            source: r#"fn main(){let x:i32=42;macro_rules! read{($arg:ident)=>{{let x:bool=true;if x{$arg}else{0}}};}let result:i32=read!(x);if result==42&&x==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            native: NativeGate::Runtime,
        },
        Case {
            name: "captured_binder_shadows_captured_uses_only_in_block",
            source: r#"fn main(){let x:i32=1;macro_rules! bind{($n:ident)=>{{let $n:bool=true;if $n{42}else{0}}};}let result:i32=bind!(x);if result+x==43{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
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
            "jr-source-macros-{}-{nonce}-{}",
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
            // Retry only the Linux pre-spawn ETXTBSY condition. Never retry an
            // executed process, nonzero exit, timeout, or different spawn error.
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
        "SOURCE MACROS BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    let mut native_partition = (0, 0, 0, 0, 0, 0);
    for case in cases() {
        match case.native {
            NativeGate::Runtime => native_partition.0 += 1,
            NativeGate::Invalid => native_partition.1 += 1,
            NativeGate::StockValidNativeUnsupportedRecursion => native_partition.2 += 1,
            NativeGate::StockValidNativeUnsupportedBlockItems => native_partition.3 += 1,
            NativeGate::StockInvalidNativeUnsupportedRecursion => native_partition.4 += 1,
            NativeGate::StockOnlyLexicalControl => native_partition.5 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=source_macros_oracle",
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
        if let Some(expected) = case.stdout {
            counts.0 += 1;
            assert!(case.stock_diagnostic.is_none());
            assert_eq!(result.code, 0, "{}:{result:?}", case.name);
            runtime(&image, expected, &scratch);
        } else {
            counts.1 += 1;
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
    assert_eq!(counts, (8, 9));
    assert_eq!(native_partition, (5, 7, 1, 2, 1, 1));
}

#[test]
fn stock_source_macro_hygiene_order_and_boundaries_edition2024() {
    stock_all();
}

#[test]
fn native_source_macros_workers1_8_check_build_no_compiler_fallback() {
    // Re-establish the independent stock classifications before native work.
    stock_all();
    let mut gates = 0;
    for case in cases() {
        if !matches!(case.native, NativeGate::Runtime | NativeGate::Invalid) {
            // This is an explicit partition, not a claimed native acceptance or
            // negative success. The stock-only controls above always execute.
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
                    i32::from(case.native == NativeGate::Invalid),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if case.native == NativeGate::Invalid {
                    assert!(case.stdout.is_none());
                    assert!(result.stdout.is_empty());
                    assert!(!diagnostic.is_empty());
                    assert!(
                        !diagnostic.to_lowercase().contains("unsupported"),
                        "{} invalid classification:{diagnostic}",
                        case.name
                    );
                }
                if let (false, Some(expected)) = (check, case.stdout) {
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
    assert_eq!(gates, 48);
}
