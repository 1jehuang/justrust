//! Nongeneric nominal unit structs, ownership and private native carrier behavior.
//! Not Rust layout/FFI/size_of compatibility or implicit Copy for zero-size types.
//! Empty record and generic units retain explicit Unsupported boundaries.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{frontend, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact pre-native JSON SHA256: stock21, then five boundary/explicit-pattern controls.
// 7fe94f26310e118c2ed521516e3cad539b7f6b85c42ac3aface051313b1aafe8
// 44d5c8c631e62932b0cc9d5322023b27bdd0053c64d2ab22f5dd3c0505a28645
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Invalid,
    Unsupported,
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
            name: "bare_constructor",
            source: r#"struct Marker;fn main(){let value:Marker=Marker;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "braced_constructor",
            source: r#"struct Marker;fn main(){let value:Marker=Marker{};println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "parenthesized_constructor_invalid",
            source: r#"struct Marker;fn main(){let value=Marker();}"#,
            stdout: None,
            stock_diagnostic: Some("expected function, found struct `Marker`"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "type_alias_bare_not_constructor",
            source: r#"struct Marker;type Alias=Marker;fn main(){let value=Alias;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0423]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "type_alias_braced_constructor",
            source: r#"struct Marker;type Alias=Marker;fn main(){let value:Alias=Alias{};println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "import_alias_constructor",
            source: r#"mod m{pub struct Marker;}use m::Marker as Alias;fn main(){let value:Alias=Alias;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "qualified_constructor",
            source: r#"mod m{pub struct Marker;}fn main(){let value:m::Marker=m::Marker;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "let_unit_pattern",
            source: r#"struct Marker;fn main(){let value=Marker;let Marker=value;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "qualified_let_unit_pattern",
            source: r#"mod m{pub struct Marker;}fn main(){let value=m::Marker;let m::Marker=value;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "match_unit_pattern",
            source: r#"struct Marker;fn main(){let value=Marker;let result=match value{Marker=>42};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "qualified_match_unit_pattern",
            source: r#"mod m{pub struct Marker;}fn main(){let value=m::Marker;let result=match value{m::Marker=>42};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_assignment_twice",
            source: r#"struct U;fn main(){let a=U;let b=a;let c=a;}"#,
            stdout: None,
            stock_diagnostic: Some("use of moved value: `a`"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wildcard_does_not_move",
            source: r#"struct U;fn main(){let a=U;let _=a;let b=a;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_let_pattern_does_not_move",
            source: r#"struct U;fn main(){let a=U;let U=a;let b=a;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_match_pattern_does_not_move",
            source: r#"struct U;fn main(){let a=U;match a{U=>{}};let b=a;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_destructure_twice",
            source: r#"struct U;fn main(){let a=U;let U=a;let U=a;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_call_twice",
            source: r#"struct U;fn consume(value:U){}fn main(){let a=U;consume(a);consume(a);}"#,
            stdout: None,
            stock_diagnostic: Some("use of moved value: `a`"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "scalar_interleaved_arguments_and_return",
            source: r#"struct Marker;fn forward(a:i32,u:Marker,b:i32,v:Marker,c:i32)->Marker{if a+b+c==42{println!("ok");}u}fn main(){let output=forward(10,Marker,20,Marker,12);}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_call_return_then_move",
            source: r#"struct Marker;fn forward(value:Marker)->Marker{value}fn main(){let input=Marker;let output=forward(input);let Marker=output;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "shared_and_mutable_references",
            source: r#"struct Marker;fn shared(value:&Marker)->i32{42}fn mutate(value:&mut Marker){*value=Marker;}fn main(){let mut value=Marker;let result=shared(&value);mutate(&mut value);if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_aggregate_zst_fields",
            source: r#"struct Marker;struct Inner{unit:Marker,value:i32}struct Outer{left:Marker,inner:Inner,right:Marker}fn forward(value:Outer)->Outer{value}fn main(){let value=forward(Outer{left:Marker,inner:Inner{unit:Marker,value:42},right:Marker});if value.inner.value==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "empty_record_boundary",
            source: r#"struct Empty{}fn main(){let value=Empty{};println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "generic_unit_boundary",
            source: r#"struct Marker<const N:usize>;fn main(){let value:Marker<1>=Marker;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "macro_scope_bare_unit_pattern_boundary",
            source: r#"struct U;fn main(){macro_rules! value{()=>{42};}let a=U;let U=a;if value!()==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "macro_scope_qualified_unit_pattern",
            source: r#"mod m{pub struct U;}fn main(){macro_rules! value{()=>{42};}let a=m::U;let m::U=a;if value!()==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_scope_braced_unit_pattern",
            source: r#"struct U;fn main(){macro_rules! value{()=>{42};}let a=U;let U{}=a;if value!()==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
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
            "jr-unit-structs-{}-{nonce}-{}",
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
        "UNIT STRUCT BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    let mut partition = (0, 0, 0);
    for case in cases() {
        match case.outcome {
            Outcome::Runtime => partition.0 += 1,
            Outcome::Invalid => partition.1 += 1,
            Outcome::Unsupported => partition.2 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=unit_structs_oracle",
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
    assert_eq!(counts, (22, 4));
    assert_eq!(partition, (19, 4, 3));
}
#[test]
fn stock_unit_struct_constructor_pattern_move_and_abi_oracle_edition2024() {
    stock_all();
}
#[test]
fn unit_source_shape_and_parser_boundaries_keep_nominal_unit_distinct_from_empty_record() {
    let mut counts = (0, 0);
    for case in cases() {
        if case.outcome == Outcome::Invalid {
            continue;
        }
        let parsed =
            source::prepare_text("unit.rs", case.source, &source::SourceOptions::default())
                .and_then(|prepared| frontend::parse_prepared(&prepared));
        if case.outcome == Outcome::Unsupported {
            if case.name == "macro_scope_bare_unit_pattern_boundary" {
                // Syntax is valid. The binder-candidate boundary belongs to
                // resolution, before any ghost binding can be published.
                let parsed = parsed.unwrap_or_else(|error| {
                    panic!("{} scoped unit pattern must parse:{error}", case.name)
                });
                let error = match frontend::resolve(&parsed) {
                    Err(error) => error,
                    Ok(_) => panic!("scoped bare unit pattern must reject during resolution"),
                };
                assert_eq!(
                    error,
                    "in function crate::main: unsupported: bare unit constructor patterns in lexical-scope functions require deferred binder classification"
                );
            } else {
                let error = match parsed {
                    Err(e) => e,
                    Ok(_) => panic!(
                        "{} explicit boundary must reject before publication",
                        case.name
                    ),
                };
                assert_eq!(
                    error.kind,
                    source::SourceErrorKind::Unsupported,
                    "{}:{error}",
                    case.name
                );
            }
            counts.1 += 1;
        } else {
            let program = parsed
                .unwrap_or_else(|e| panic!("{} supported unit syntax must parse:{e}", case.name))
                .program;
            assert!(
                program
                    .structs
                    .iter()
                    .any(|decl| decl.shape == frontend::StructShape::Unit),
                "{} unit source shape lost",
                case.name
            );
            for decl in program
                .structs
                .iter()
                .filter(|decl| decl.shape == frontend::StructShape::Unit)
            {
                assert!(decl.fields.is_empty());
                assert!(decl.type_params.is_empty());
            }
            counts.0 += 1;
        }
    }
    assert_eq!(counts, (19, 3));
}
#[test]
fn native_unit_structs_workers1_8_check_build_no_compiler_fallback() {
    // jr-rust --profile emits telemetry, not optimization. The coordinator runs
    // this same target under debug and release BOOTSTRAP compiler profiles.
    // Neither run asserts a nonexistent optimized native code-generation mode.
    stock_all();
    let mut gates = 0;
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
                    if case.outcome == Outcome::Unsupported {
                        assert!(
                            diagnostic.contains("unsupported:"),
                            "{} explicit boundary:{diagnostic}",
                            case.name
                        );
                    } else {
                        assert!(
                            !diagnostic.contains("unsupported:"),
                            "{} ordinary invalid must not become Unsupported:{diagnostic}",
                            case.name
                        );
                    }
                }
                if !check && case.outcome == Outcome::Runtime {
                    runtime(
                        &image,
                        case.stdout.expect("runtime source oracle"),
                        &scratch,
                    );
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
                    (Outcome::Invalid, true) => modes.2 += 1,
                    (Outcome::Invalid, false) => modes.3 += 1,
                    (Outcome::Unsupported, true) => modes.4 += 1,
                    (Outcome::Unsupported, false) => modes.5 += 1,
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 104);
    assert_eq!(modes, (38, 38, 8, 8, 6, 6));
    eprintln!(
        "NATIVE UNIT STRUCTS:104 cells, modes{modes:?};38 executed native images;12 Unsupported cells are boundaries, not Rust-invalid parity or native layout/FFI proof"
    );
}
