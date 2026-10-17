//! Nonempty nongeneric tuple structs with checked nominal field identities.
//! Not first-class constructors, tuple aggregate match, generics, or Rust ABI/layout.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact36 pre-native sources, hashes, diagnostics and runtime stock JSON SHA256:
// 12524b217f2e02e14a0a594adff9c7fb65397febedca4ba76d06017cb100ce4e
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
            name: "tuple_constructor_bare_binding_shadow",
            source: r#"struct Pair(i32);fn main(){let Pair=7;println!("ok");}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0530]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_constructor_mut_binding_shadow",
            source: r#"struct Pair(i32);fn main(){let mut Pair=7;println!("ok");}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0530]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_constructor_parameter_shadow",
            source: r#"struct Pair(i32);fn f(Pair:i32)->i32{Pair}fn main(){println!("ok");}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0530]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_import_alias_binding_shadow",
            source: r#"struct Pair(i32);use Pair as P;fn main(){let P=7;println!("ok");}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0530]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "record_type_name_binding",
            source: r#"struct Record{x:i32}fn main(){let Record=42;if Record==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "type_alias_name_binding",
            source: r#"struct Pair(i32);type Alias=Pair;fn main(){let Alias=42;if Alias==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_constructor_and_fields",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);if value.0+value.1==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_let_destructure",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let Pair(a,b)=value;if a+b==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "qualified_tuple_let_destructure",
            source: r#"mod m{pub struct Pair(pub i32,pub i32);}fn main(){let value=m::Pair(40,2);let m::Pair(a,b)=value;if a+b==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "import_alias_constructor",
            source: r#"mod m{pub struct Pair(pub i32,pub i32);}use m::Pair as Alias;fn main(){let value=Alias(40,2);if value.0+value.1==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "type_alias_not_constructor",
            source: r#"struct Pair(i32);type Alias=Pair;fn main(){let value=Alias(42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0423]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_zero_arguments_wrong_arity",
            source: r#"struct Pair(i32);fn main(){let value=Pair();}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0061]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_extra_arguments_wrong_arity",
            source: r#"struct Pair(i32);fn main(){let value=Pair(40,2);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0061]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "record_dot_zero_invalid",
            source: r#"struct Record{x:i32}fn main(){let value=Record{x:42};let field=value.0;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0609]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_field_out_of_range",
            source: r#"struct Pair(i32);fn main(){let value=Pair(42);let field=value.1;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0609]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_private_constructor_field",
            source: r#"mod m{pub struct Pair(i32);}fn main(){let value=m::Pair(42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0603]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_private_second_constructor_field",
            source: r#"mod m{pub struct Pair(pub i32,i32);}fn main(){let value=m::Pair(40,2);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0603]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_private_field_after_public_factory",
            source: r#"mod m{pub struct Pair(i32);pub fn make()->Pair{Pair(42)}}fn main(){let value=m::make();let field=value.0;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0616]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_public_field_after_factory",
            source: r#"mod m{pub struct Pair(pub i32);pub fn make()->Pair{Pair(42)}}fn main(){let value=m::make();if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_private_type_factory",
            source: r#"mod m{struct Pair(pub i32);pub fn make()->Pair{Pair(42)}}fn main(){let value=m::make();}"#,
            stdout: None,
            stock_diagnostic: Some("type `Pair` is private"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_nominal_pattern_mismatch",
            source: r#"struct A(i32);struct B(i32);fn main(){let A(x)=B(42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_noncopy_assignment_twice",
            source: r#"struct Pair(i32);fn main(){let a=Pair(42);let b=a;let c=a;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_noncopy_call_twice",
            source: r#"struct Pair(i32);fn consume(value:Pair){}fn main(){let a=Pair(42);consume(a);consume(a);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_scalar_destructure_preserves_value",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let Pair(a,b)=value;let next=value;if a+b+next.0==82{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_nested_noncopy_destructure_moves_field",
            source: r#"struct Inner(i32);struct Pair(Inner,i32);fn main(){let value=Pair(Inner(40),2);let Pair(inner,n)=value;let next=value;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_nested_aggregate_and_return",
            source: r#"struct Pair(i32,i32);struct Outer{pair:Pair,tail:i32}fn forward(value:Outer)->Outer{value}fn main(){let value=forward(Outer{pair:Pair(20,20),tail:2});if value.pair.0+value.pair.1+value.tail==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_interleaved_arguments_and_return",
            source: r#"struct Pair(i32);fn forward(a:i32,pair:Pair,b:i32)->Pair{Pair(a+pair.0+b)}fn main(){let value=forward(10,Pair(20),12);if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_macro_scope_explicit_let_pattern",
            source: r#"struct Pair(i32);fn main(){macro_rules! n{()=>{42};}let Pair(value)=Pair(n!());if value==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_match_boundary",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let result=match value{Pair(a,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "tuple_numeric_braced_constructor_boundary",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair{0:40,1:2};if value.0+value.1==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "tuple_zero_field_boundary",
            source: r#"struct Empty();fn main(){let value=Empty();println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "tuple_generic_boundary",
            source: r#"struct Wrap<T>(T);fn main(){let value=Wrap(42);if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "tuple_first_class_constructor_boundary",
            source: r#"struct Pair(i32);fn main(){let constructor=Pair;let value=constructor(42);if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "tuple_constructor_parameter_pattern_boundary",
            source: r#"struct Pair(i32);fn read(Pair(value):Pair)->i32{value}fn main(){if read(Pair(42))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "tuple_repr_boundary",
            source: r#"#[repr(C)]struct Pair(i32);fn main(){let value=Pair(42);if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "tuple_derive_boundary",
            source: r#"#[derive(Clone,Copy)]struct Pair(i32);fn main(){let value=Pair(42);let copy=value;if value.0==copy.0{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
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
            "jr-tuple-structs-{}-{nonce}-{}",
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
        "TUPLE STRUCT BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=tuple_structs_oracle",
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
    assert_eq!(counts, (19, 17));
    assert_eq!(partition, (11, 17, 8));
}
#[test]
fn stock_tuple_struct_constructor_pattern_move_privacy_oracle_edition2024() {
    stock_all();
}
#[test]
fn tuple_source_shapes_use_numeric_fields_and_boundaries_keep_their_actual_phase() {
    let mut counts = (0, 0, 0, 0);
    for case in cases() {
        if case.outcome == Outcome::Invalid {
            continue;
        }
        let parsed =
            source::prepare_text("tuple.rs", case.source, &source::SourceOptions::default())
                .and_then(|prepared| frontend::parse_prepared(&prepared));
        if case.outcome == Outcome::Runtime {
            let parsed = parsed.unwrap_or_else(|error| {
                panic!("{} supported tuple source must parse:{error}", case.name)
            });
            let program = frontend::resolve(&parsed).unwrap_or_else(|error| {
                panic!("{} supported tuple source must resolve:{error}", case.name)
            });
            let tuples: Vec<_> = program
                .structs
                .iter()
                .filter(|decl| decl.shape == frontend::StructShape::Tuple)
                .collect();
            if case.name == "record_type_name_binding" {
                assert!(tuples.is_empty());
            } else {
                assert!(!tuples.is_empty(), "{} tuple source shape lost", case.name);
            }
            for decl in tuples {
                assert!(!decl.fields.is_empty());
                assert!(decl.type_params.is_empty());
                for (index, field) in decl.fields.iter().enumerate() {
                    assert!(
                        matches!(field.name,frontend::FieldName::Tuple(actual) if actual==index),
                        "{} tuple field must retain structural index {index}",
                        case.name
                    );
                }
            }
            counts.0 += 1;
        } else if case.name == "tuple_first_class_constructor_boundary" {
            let parsed = parsed
                .unwrap_or_else(|error| panic!("first-class constructor is valid syntax:{error}"));
            let error = match frontend::resolve(&parsed) {
                Err(error) => error,
                Ok(_) => panic!("first-class tuple constructor must reject during resolution"),
            };
            assert!(
                error
                    .contains("unsupported: first-class tuple struct constructors are unsupported"),
                "{error}"
            );
            counts.2 += 1;
        } else if case.name == "tuple_match_boundary" {
            let parsed =
                parsed.unwrap_or_else(|error| panic!("tuple aggregate match must parse:{error}"));
            frontend::resolve(&parsed).unwrap_or_else(|error| {
                panic!("tuple aggregate match must resolve before checker boundary:{error}")
            });
            let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(error) => error,
                Ok(_) => panic!("tuple aggregate match must reject before executable publication"),
            };
            assert!(
                error
                    .to_string()
                    .contains("unsupported: tuple struct match patterns"),
                "{error}"
            );
            counts.3 += 1;
        } else {
            let error = match parsed {
                Err(error) => error,
                Ok(_) => panic!(
                    "{} source admission boundary must reject before resolution",
                    case.name
                ),
            };
            assert_eq!(
                error.kind,
                source::SourceErrorKind::Unsupported,
                "{}:{error}",
                case.name
            );
            counts.1 += 1;
        }
    }
    assert_eq!(counts, (11, 6, 1, 1));
}
#[test]
fn native_tuple_structs_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(gates, 144);
    assert_eq!(modes, (22, 22, 34, 34, 16, 16));
    eprintln!(
        "NATIVE TUPLE STRUCTS:144 cells, modes{modes:?};22 executed native images;32 Unsupported cells are boundaries, not Rust-invalid parity or native layout/FFI proof"
    );
}
