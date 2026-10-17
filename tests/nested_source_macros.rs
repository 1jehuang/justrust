//! Original-source local macro definitions and bounded nested expression expansion.
//! No generated definitions, item/path/export macros, or general hygiene claim.
//! Exact stock sources were measured before writing native expectations.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler::{frontend, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Pre-native JSON contains all17 exact source bytes, hashes and full diagnostics:
// bf264bf01f71a2b2eb7cc7133ee9c49aeef5c702ca7a168c4e7605e962c3e701
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Syntax,
    Type,
    Limit,
}
struct Case {
    name: &'static str,
    source: &'static str,
    stdout: Option<&'static str>,
    stock_diagnostic: Option<&'static str>,
    outcome: Outcome,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "two_direct_macro_names",
            source: r#"fn main(){macro_rules! a{()=>{40};}macro_rules! b{()=>{5};}if a!()+b!()==45{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "same_name_source_order_redefinition",
            source: r#"fn main(){macro_rules! a{()=>{40};}let before:i32=a!();macro_rules! a{()=>{5};}let after:i32=a!();if before+after==45{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_block_shadow_restore",
            source: r#"fn main(){macro_rules! a{()=>{40};}let before:i32=a!();{macro_rules! a{()=>{5};}let inside:i32=a!();if inside==5{println!("inner");}}if before+a!()==80{println!("outer");}}"#,
            stdout: Some("inner\nouter\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "block_definition_scope_escape",
            source: r#"fn main(){{macro_rules! read{()=>{40};}}let result:i32=read!();}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `read`"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "direct_call_before_definition",
            source: r#"fn main(){let result:i32=read!();macro_rules! read{()=>{40};}}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `read`"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "first_matched_arm_type_failure_no_fallback",
            source: r#"fn main(){macro_rules! choose{($arg:ident)=>{true+1};($arg:tt)=>{42};}let result:i32=choose!(name);}"#,
            stdout: None,
            stock_diagnostic: Some("cannot add"),
            outcome: Outcome::Type,
        },
        Case {
            name: "opaque_literal_forward_42",
            source: r#"fn main(){macro_rules! sink{(42)=>{1};($x:literal)=>{2};}macro_rules! pass{($x:literal)=>{sink!($x)};}let result:i32=pass!(42);if result==2{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "transparent_tt_forward_42",
            source: r#"fn main(){macro_rules! sink{(42)=>{1};($x:literal)=>{2};}macro_rules! pass{($x:tt)=>{sink!($x)};}let result:i32=pass!(42);if result==1{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "opaque_negative_literal_forward",
            source: r#"fn main(){macro_rules! sink{(-42)=>{1};($x:literal)=>{2};}macro_rules! pass{($x:literal)=>{sink!($x)};}let result:i32=pass!(-42);if result==2{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_captured_invocation_local",
            source: r#"fn main(){let x:i32=40;macro_rules! leaf{($a:ident)=>{x+$a};}macro_rules! outer{($a:ident)=>{leaf!($a)};}{let x:i32=1;let y:i32=3;let answer:i32=outer!(y);if answer==43{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_two_definition_local_anchors",
            source: r#"fn main(){let x:i32=40;macro_rules! sink{($arg:ident)=>{x+$arg};}{let x:i32=1;macro_rules! pass{()=>{sink!(x)};}{let x:i32=2;let result:i32=pass!();if result==41{println!("ok");}else{println!("bad");}}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_macro_lookup_after_redefinition",
            source: r#"fn main(){macro_rules! leaf{()=>{40};}macro_rules! outer{()=>{leaf!()};}macro_rules! leaf{()=>{5};}let result:i32=outer!();if result==5{println!("call");}else{if result==40{println!("definition");}else{println!("bad");}}}"#,
            stdout: Some("call\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "outer_definition_before_leaf_call_after_leaf",
            source: r#"fn main(){macro_rules! outer{()=>{leaf!()};}macro_rules! leaf{()=>{42};}let result:i32=outer!();if result==42{println!("ok");}else{println!("bad");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "outer_call_before_leaf_definition",
            source: r#"fn main(){macro_rules! outer{()=>{leaf!()};}let result:i32=outer!();macro_rules! leaf{()=>{42};}}"#,
            stdout: None,
            stock_diagnostic: Some("cannot find macro `leaf`"),
            outcome: Outcome::Syntax,
        },
        Case {
            name: "terminating_token_munch_countdown",
            source: r#"fn main(){macro_rules! walk{()=>{40};(@ $($rest:tt)*)=>{walk!($($rest)*)+1};}if walk!(@ @ @)==43{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unbounded_nested_recursion",
            source: r#"fn main(){macro_rules! again{()=>{again!()};}let result:i32=again!();}"#,
            stdout: None,
            stock_diagnostic: Some("recursion limit reached"),
            outcome: Outcome::Limit,
        },
        Case {
            name: "opaque_literal_cannot_match_fixed_only",
            source: r#"fn main(){macro_rules! sink{(42)=>{1};}macro_rules! pass{($x:literal)=>{sink!($x)};}let result:i32=pass!(42);}"#,
            stdout: None,
            stock_diagnostic: Some("no rules expected"),
            outcome: Outcome::Syntax,
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
            "jr-nested-source-macros-{}-{nonce}-{}",
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
            // Retry only Linux pre-spawn ETXTBSY, never an executed child.
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
        "NESTED SOURCE BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    let mut partition = (0, 0, 0, 0);
    for case in cases() {
        match case.outcome {
            Outcome::Runtime => partition.0 += 1,
            Outcome::Syntax => partition.1 += 1,
            Outcome::Type => partition.2 += 1,
            Outcome::Limit => partition.3 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=nested_source_macros_oracle",
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
    assert_eq!(counts, (11, 6));
    assert_eq!(partition, (11, 4, 1, 1));
}

#[test]
fn stock_multiple_original_definitions_and_nested_expression_oracle_edition2024() {
    stock_all();
}

#[test]
fn nested_source_errors_keep_typed_resource_and_syntax_categories_and_committed_arm() {
    let mut counts = (0, 0, 0);
    for case in cases() {
        if case.outcome == Outcome::Runtime {
            continue;
        }
        let parsed =
            source::prepare_text("nested.rs", case.source, &source::SourceOptions::default())
                .and_then(|prepared| frontend::parse_prepared(&prepared));
        match case.outcome {
            Outcome::Type => {
                // The first matched arm's true+1 is admitted syntax. Its later
                // type rejection cannot cause the matcher to try the 42 arm.
                parsed.unwrap_or_else(|error| {
                    panic!("{} must parse before type rejection:{error}", case.name)
                });
                counts.1 += 1;
            }
            Outcome::Syntax | Outcome::Limit => {
                let error = match parsed {
                    Err(error) => error,
                    Ok(_) => panic!("{} must reject before publication", case.name),
                };
                let expected = if case.outcome == Outcome::Limit {
                    counts.2 += 1;
                    source::SourceErrorKind::Limit
                } else {
                    counts.0 += 1;
                    source::SourceErrorKind::Syntax
                };
                assert_eq!(error.kind, expected, "{}:{error}", case.name);
            }
            Outcome::Runtime => unreachable!(),
        }
    }
    assert_eq!(counts, (4, 1, 1));
}

#[test]
fn native_nested_source_macros_workers1_8_check_build_no_compiler_fallback() {
    stock_all();
    let mut gates = 0;
    // Successful runtime-source CHECK, BUILD+execution, ordinary invalid CHECK,
    // ordinary invalid BUILD, resource CHECK and resource BUILD respectively.
    let mut modes = (0, 0, 0, 0, 0, 0);
    for case in cases() {
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
                    i32::from(case.outcome != Outcome::Runtime),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if case.outcome != Outcome::Runtime {
                    assert!(result.stdout.is_empty());
                    assert!(!diagnostic.is_empty());
                    // Resource classification is pinned by SourceErrorKind in
                    // the public parser test above, never guessed from prose.
                    if case.outcome != Outcome::Limit {
                        assert!(
                            !diagnostic.contains("unsupported:"),
                            "{} ordinary invalid must not become Unsupported:{diagnostic}",
                            case.name
                        );
                    }
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
                match (case.outcome, check) {
                    (Outcome::Runtime, true) => modes.0 += 1,
                    (Outcome::Runtime, false) => modes.1 += 1,
                    (Outcome::Syntax | Outcome::Type, true) => modes.2 += 1,
                    (Outcome::Syntax | Outcome::Type, false) => modes.3 += 1,
                    (Outcome::Limit, true) => modes.4 += 1,
                    (Outcome::Limit, false) => modes.5 += 1,
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 68);
    assert_eq!(modes, (22, 22, 10, 10, 2, 2));
    eprintln!("NATIVE NESTED SOURCE:68 cells, modes{modes:?};22 executed native images");
}
