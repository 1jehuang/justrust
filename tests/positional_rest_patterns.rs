//! Positional rest over exact nonempty tuples and nongeneric tuple structs.
//! Empty tuples, enum tuple payload rest, and advanced patterns remain boundaries.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact pre-native24-source stock JSON SHA256:
// e806ccf8a5e28cf878f60cad61d835ed4226e898cdac0d780dab489300ba1324
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
            name: "tuple_middle",
            source: r#"fn main(){let value=(40,7,2);let result=match value{(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_leading",
            source: r#"fn main(){let value=(7,40,2);let result=match value{(..,a,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_trailing",
            source: r#"fn main(){let value=(40,2,7);let result=match value{(a,b,..)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_rest_zero_width",
            source: r#"fn main(){let value=(40,2);let result=match value{(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_all_rest_noncopy",
            source: r#"struct Inner{x:i32}fn main(){let value=(Inner{x:40},2);let result=match value{(..)=>2};let next=value;if next.0.x+result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_struct_middle",
            source: r#"struct Triple(i32,i32,i32);fn main(){let value=Triple(40,7,2);let result=match value{Triple(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_struct_let_suffix",
            source: r#"struct Triple(i32,i32,i32);fn main(){let value=Triple(7,40,2);let Triple(..,a,b)=value;if a+b==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_omitted_noncopy_reuse",
            source: r#"struct Inner{x:i32}fn main(){let value=(40,Inner{x:7},2);let result=match value{(a,..,b)=>a+b};let next=value;if result==42{if next.1.x==7{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_selected_noncopy_move",
            source: r#"struct Inner{x:i32}fn main(){let value=(Inner{x:40},7,2);let result=match value{(inner,..)=>inner};let next=value;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "too_many_explicit",
            source: r#"fn main(){let value=(40,2);let result=match value{(a,b,..,c)=>a+b+c};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "duplicate_rest",
            source: r#"fn main(){let value=(40,7,2);let result=match value{(a,..,..,b)=>a+b};}"#,
            stdout: None,
            stock_diagnostic: Some("`..` can only be used once per tuple pattern"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_nominal",
            source: r#"struct A(i32,i32);struct B(i32,i32);fn main(){let value=B(40,2);let result=match value{A(a,..)=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "private_selected_suffix",
            source: r#"mod m{pub struct Triple(pub i32,pub i32,i32);pub fn make()->Triple{Triple(40,7,2)}}fn main(){let result=match m::make(){m::Triple(..,n)=>n};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0603]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "private_omitted_middle",
            source: r#"mod m{pub struct Triple(pub i32,i32,pub i32);pub fn make()->Triple{Triple(40,7,2)}}fn main(){let result=match m::make(){m::Triple(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0603]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "enum_tuple_middle",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let result=match Item::Only(40,7,2){Item::Only(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "empty_tuple_rest",
            source: r#"fn main(){let result=match (){(..)=>42};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "nested_positional_rest",
            source: r#"struct Triple(i32,i32,i32);fn main(){let value=(Triple(40,7,2),9);let result=match value{(Triple(a,..,b),..)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_captured_rest_binder",
            source: r#"fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{match $p{(..,$n)=>$n}}};}let value=(7,42);let answer=pick!(value,n);if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only_tuple_scrutinee",
            source: r#"fn make()->(i32,i32,i32){println!("once");(40,7,2)}fn main(){let result=match make(){(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_struct_allrest_reuse",
            source: r#"struct Inner{x:i32}struct Pair(Inner,i32);fn main(){let value=Pair(Inner{x:40},2);let result=match value{Pair(..)=>2};let next=value;if next.0.x+result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "duplicate_binding",
            source: r#"fn main(){let value=(40,7,2);let result=match value{(a,..,a)=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0416]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "guard_boundary",
            source: r#"fn main(){let value=(40,7,2);let result=match value{(a,..,b) if a==40=>a+b,_=>0};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "ref_binding_boundary",
            source: r#"fn main(){let value=(40,7,2);let result=match value{(ref a,..,b)=>*a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "refutable_leaf_boundary",
            source: r#"fn main(){let value=(40,7,2);let result=match value{(40,..,b)=>40+b,_=>0};if result==42{println!("ok");}}"#,
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
            "jr-positional-rest-{}-{nonce}-{}",
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
        "POSITIONAL REST BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=positional_rest_oracle",
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
    assert_eq!(counts, (17, 7));
    assert_eq!(partition, (12, 7, 5));
}
#[test]
fn stock_positional_rest_mapping_privacy_ownership_oracle_edition2024() {
    stock_all();
}
fn typed_main(case: &Case) -> ir::CheckedFunction {
    let program = frontend::parse(case.source)
        .unwrap_or_else(|error| panic!("{} parse/resolve:{error}", case.name));
    let signatures = frontend::signature_table(&program)
        .unwrap_or_else(|error| panic!("{} signatures:{error}", case.name));
    let index = program
        .functions
        .iter()
        .position(|function| {
            function.name.check(program.names.as_ref()).unwrap();
            function.name.anchor() == frontend::PathAnchor::Crate
                && function.name.len() == 1
                && program.names.spelling(function.name.segments()[0]).unwrap() == "main"
        })
        .expect("checked root main");
    frontend::type_function_with_signatures(&program, index, &signatures)
        .unwrap_or_else(|error| panic!("{} checking:{error}", case.name))
}
#[test]
fn source_positional_rest_ir_uses_actual_field_indexes_without_omitted_bindings() {
    let mut counts = (0, 0);
    for case in cases()
        .into_iter()
        .filter(|case| case.outcome == Outcome::Runtime)
    {
        let function = typed_main(&case);
        function
            .validate()
            .unwrap_or_else(|error| panic!("{} invalid publishedIR:{error}", case.name));
        let mut aggregate_arms = 0;
        let mut seen = std::collections::HashSet::new();
        for expression in &function.exprs {
            if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                for arm in arms {
                    if let ir::MatchPattern::Aggregate { ty, bindings } = &arm.pattern {
                        match ty {
                            ir::Ty::Struct(_) => {}
                            ir::Ty::Tuple(fields) => assert!(!fields.is_empty()),
                            _ => panic!("{} invalidaggregate domain:{ty:?}", case.name),
                        }
                        for binding in bindings {
                            assert!(!binding.projection.is_empty());
                            assert!(
                                binding
                                    .projection
                                    .iter()
                                    .all(|step| matches!(step, ir::Projection::Field(_))),
                                "{} aggregate path must never carry enumtag/downcast/deref/index",
                                case.name
                            );
                            assert!(!function.params.contains(&binding.local));
                            assert!(
                                seen.insert(binding.local),
                                "{} aggregate binding local repeated",
                                case.name
                            );
                        }
                        aggregate_arms += 1;
                    }
                }
            }
        }
        if case.name == "tuple_struct_let_suffix" {
            assert_eq!(
                aggregate_arms, 0,
                "let destructuring is not a fabricated match arm"
            );
        } else {
            assert_eq!(
                aggregate_arms, 1,
                "{} expected one genuine aggregate arm",
                case.name
            );
            let expected: Vec<Vec<usize>> = match case.name {
                "tuple_middle"
                | "tuple_leading"
                | "tuple_trailing"
                | "tuple_struct_middle"
                | "tuple_omitted_noncopy_reuse"
                | "once_only_tuple_scrutinee" => {
                    let (a, b) = match case.name {
                        "tuple_leading" => (1, 2),
                        "tuple_trailing" => (0, 1),
                        _ => (0, 2),
                    };
                    vec![vec![a], vec![b]]
                }
                "tuple_rest_zero_width" => vec![vec![0], vec![1]],
                "tuple_all_rest_noncopy" | "tuple_struct_allrest_reuse" => vec![],
                "nested_positional_rest" => vec![vec![0, 0], vec![0, 2]],
                "macro_captured_rest_binder" => vec![vec![1]],
                _ => unreachable!(),
            };
            let mut actual = Vec::new();
            for expression in &function.exprs {
                if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                    for arm in arms {
                        if let ir::MatchPattern::Aggregate { bindings, .. } = &arm.pattern {
                            for binding in bindings {
                                actual.push(
                                    binding
                                        .projection
                                        .iter()
                                        .map(|step| match step {
                                            ir::Projection::Field(index) => *index,
                                            _ => panic!("nonfield aggregate projection"),
                                        })
                                        .collect::<Vec<_>>(),
                                );
                            }
                        }
                    }
                }
            }
            assert_eq!(
                actual, expected,
                "{} prefix/suffix must map to exact actual fields",
                case.name
            );
            counts.1 += 1;
        }
        counts.0 += 1;
    }
    assert_eq!(counts, (12, 11));
}

#[test]
fn positional_rest_closed_domains_reject_in_their_actual_frontend_phase() {
    let mut counts = (0, 0, 0);
    for case in cases()
        .into_iter()
        .filter(|case| case.outcome == Outcome::Unsupported)
    {
        let parsed = source::prepare_text(
            "positional.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .and_then(|prepared| frontend::parse_prepared(&prepared));
        if matches!(case.name, "guard_boundary" | "ref_binding_boundary") {
            let error = match parsed {
                Err(error) => error,
                Ok(_) => panic!("{} must reject as explicit parser Unsupported", case.name),
            };
            assert_eq!(
                error.kind,
                source::SourceErrorKind::Unsupported,
                "{}:{error}",
                case.name
            );
            counts.0 += 1;
        } else {
            let parsed = parsed.unwrap_or_else(|error| {
                panic!(
                    "{} valid pattern grammar must parse before checker boundary:{error}",
                    case.name
                )
            });
            if case.name == "enum_tuple_middle" {
                let error = match frontend::resolve(&parsed) {
                    Err(error) => error,
                    Ok(_) => {
                        panic!("enum tuple rest must reject in source resolver before publication")
                    }
                };
                assert!(
                    error.contains(
                        "unsupported: enum tuple payload positional rest patterns are unsupported"
                    ),
                    "{error}"
                );
                counts.2 += 1;
                continue;
            }
            frontend::resolve(&parsed).unwrap_or_else(|error| {
                panic!("{} must resolve before checker boundary:{error}", case.name)
            });
            let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(error) => error,
                Ok(_) => panic!("{} must reject before executable publication", case.name),
            };
            let expected = match case.name {
                "refutable_leaf_boundary" => "unsupported: refutable aggregate match leaf",
                "empty_tuple_rest" => "unsupported: positional rest requires a nonempty tuple",
                _ => unreachable!(),
            };
            assert!(
                failure.to_string().contains(expected),
                "{}:{failure}",
                case.name
            );
            counts.1 += 1;
        }
    }
    assert_eq!(counts, (2, 2, 1));
}
#[test]
fn native_positional_rest_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(gates, 96);
    assert_eq!(modes, (24, 24, 14, 14, 10, 10));
    eprintln!(
        "NATIVE POSITIONAL REST:96 cells, modes{modes:?};24 executed native images;20 Unsupported cells are boundaries, not Rust-invalid parity or native layout/FFI proof"
    );
}
