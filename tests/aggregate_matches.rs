//! Single-arm irrefutable aggregate matches, not general Rust pattern support.
//! Refutable, guard/or/ref/ergonomic/enum-child patterns remain boundaries.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact pre-native26-source stock JSON SHA256:
// ba893b737576525f3dcc93b71553fc7c39d0e6cd2afc99d8b908cb94490c725b
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
            name: "tuple_struct_match",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let result=match value{Pair(a,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "record_match",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let result=match value{Pair{a,b}=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "structural_tuple_match",
            source: r#"fn main(){let value=(40,2);let result=match value{(a,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_mixed_aggregate_match",
            source: r#"struct Pair(i32,i32);struct Packet{pair:Pair,tail:(i32,i32)}fn main(){let value=Packet{pair:Pair(20,20),tail:(1,1)};let result=match value{Packet{pair:Pair(a,b),tail:(c,d)}=>a+b+c+d};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "qualified_tuple_struct_match",
            source: r#"mod m{pub struct Pair(pub i32,pub i32);}fn main(){let value=m::Pair(40,2);let result=match value{m::Pair(a,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "record_fields_reordered",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let result=match value{Pair{b:y,a:x}=>x+y};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "scalar_copy_fields_preserve_scrutinee",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let result=match value{Pair(a,b)=>a+b};let next=value;if result+next.0==82{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wildcards_preserve_noncopy_fields",
            source: r#"struct Inner(i32);struct Pair(Inner,i32);fn main(){let value=Pair(Inner(40),2);let result=match value{Pair(_,n)=>n};let next=value;if next.0.0+result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_binding_return",
            source: r#"struct Inner(i32);struct Pair(Inner,i32);fn main(){let value=Pair(Inner(42),2);let inner=match value{Pair(inner,_)=>inner};if inner.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only_scrutinee",
            source: r#"struct Pair(i32,i32);fn make()->Pair{println!("once");Pair(40,2)}fn main(){let result=match make(){Pair(a,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "whole_aggregate_binding_control",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let result=match value{whole=>whole.0+whole.1};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_match_control",
            source: r#"fn main(){let result=match (){()=>42};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_noncopy_binding_moves_scrutinee",
            source: r#"struct Inner(i32);struct Pair(Inner,i32);fn main(){let value=Pair(Inner(40),2);let result=match value{Pair(inner,n)=>inner.0+n};let next=value;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "nominal_pattern_mismatch",
            source: r#"struct A(i32);struct B(i32);fn main(){let value=B(42);let result=match value{A(x)=>x};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_pattern_wrong_arity",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let result=match value{Pair(x)=>x};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0023]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "record_pattern_missing_field",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let result=match value{Pair{a}=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0027]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "private_tuple_field_pattern",
            source: r#"mod m{pub struct Pair(i32);pub fn make()->Pair{Pair(42)}}fn main(){let result=match m::make(){m::Pair(x)=>x};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0603]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "private_record_field_pattern",
            source: r#"mod m{pub struct Pair{a:i32}pub fn make()->Pair{Pair{a:42}}}fn main(){let result=match m::make(){m::Pair{a}=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0451]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "duplicate_pattern_binding",
            source: r#"struct Pair(i32,i32);fn main(){let result=match Pair(40,2){Pair(x,x)=>x};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0416]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "refutable_literal_boundary",
            source: r#"struct Pair(i32,i32);fn main(){let result=match Pair(40,2){Pair(40,b)=>40+b,_=>0};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "rest_pattern_boundary",
            source: r#"struct Pair(i32,i32);fn main(){let result=match Pair(42,2){Pair(a,..)=>a};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "ref_binding_boundary",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let result=match value{Pair(ref a,b)=>*a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "reference_match_ergonomics_boundary",
            source: r#"struct Pair(i32,i32);fn main(){let value=Pair(40,2);let result=match &value{Pair(a,b)=>*a+*b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_enum_child_boundary",
            source: r#"enum Item{Only(i32)}struct Packet(Item);fn main(){let result=match Packet(Item::Only(42)){Packet(Item::Only(x))=>x};if result==42{println!("ok");}}"#,
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
            "jr-aggregate-matches-{}-{nonce}-{}",
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
        "AGGREGATE MATCH BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=aggregate_matches_oracle",
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
    assert_eq!(partition, (17, 7, 0));
}
#[test]
fn stock_aggregate_matches_irrefutability_privacy_ownership_oracle_edition2024() {
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
fn source_aggregate_match_ir_has_checked_domains_and_only_plain_field_binding_paths() {
    let mut counts = (0, 0);
    for case in cases().into_iter().filter(|case| {
        case.outcome == Outcome::Runtime
            && !matches!(
                case.name,
                "refutable_literal_boundary" | "nested_enum_child_boundary"
            )
    }) {
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
                        let ergonomic = case.name == "reference_match_ergonomics_boundary";
                        match ty {
                            ir::Ty::Struct(_) => {}
                            ir::Ty::Tuple(fields) => assert!(!fields.is_empty()),
                            ir::Ty::Ref { .. } if ergonomic => {}
                            _ => panic!("{} invalidaggregate domain:{ty:?}", case.name),
                        }
                        for binding in bindings {
                            assert!(!binding.projection.is_empty());
                            // Default binding modes: one leading deref, then
                            // plain fields, bound by shared reference.
                            let fields = if ergonomic {
                                assert_eq!(binding.projection[0], ir::Projection::Deref);
                                assert_eq!(binding.borrow, Some(ir::BorrowKind::Shared));
                                &binding.projection[1..]
                            } else {
                                &binding.projection[..]
                            };
                            assert!(
                                fields
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
        if matches!(
            case.name,
            "whole_aggregate_binding_control" | "unit_match_control"
        ) {
            assert_eq!(aggregate_arms, 0);
            counts.1 += 1;
        } else {
            assert_eq!(
                aggregate_arms, 1,
                "{} expected one genuine aggregate arm",
                case.name
            );
            counts.0 += 1;
        }
    }
    assert_eq!(counts, (13, 2));
}
#[test]
fn aggregate_advanced_pattern_boundaries_reject_in_their_actual_frontend_phase() {
    // Counts preserve parser, checker and resolver rejection phases separately.
    let mut counts = (0, 0, 0);
    for case in cases()
        .into_iter()
        .filter(|case| case.outcome == Outcome::Unsupported)
    {
        let parsed = source::prepare_text(
            "aggregate.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .and_then(|prepared| frontend::parse_prepared(&prepared));
        if case.name == "or_pattern_boundary" {
            let parsed = parsed.expect("original root OR grammar must parse");
            let error = match frontend::resolve(&parsed) {
                Err(error) => error,
                Ok(_) => panic!("{} binding OR must reject in resolver", case.name),
            };
            assert!(
                error
                    .to_string()
                    .contains("unsupported: or-pattern bindings are unsupported"),
                "{}:{error}",
                case.name
            );
            counts.2 += 1;
        } else if case.name == "guard_boundary" {
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
            frontend::resolve(&parsed).unwrap_or_else(|error| {
                panic!("{} must resolve before checker boundary:{error}", case.name)
            });
            let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(error) => error,
                Ok(_) => panic!("{} must reject before executable publication", case.name),
            };
            let expected = match case.name {
                "refutable_literal_boundary" => "unsupported: refutable aggregate match leaf",
                "nested_enum_child_boundary" => {
                    "unsupported: enum children in aggregate match patterns"
                }
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
    assert_eq!(counts, (0, 0, 0));
}
#[test]
fn native_aggregate_matches_workers1_8_check_build_no_compiler_fallback() {
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
                let result = capture(&mut command, &scratch, 180);
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
    assert_eq!(modes, (34, 34, 14, 14, 0, 0));
    eprintln!(
        "NATIVE AGGREGATE MATCHES:104 cells, modes{modes:?};34 executed native images;8 Unsupported cells are boundaries, not Rust-invalid parity or native layout/FFI proof"
    );
}

#[test]
fn retained_newly_supported_decision_witnesses_publish_real_checked_predicates() {
    let mut sources = 0;
    for case in cases().into_iter().filter(|case| {
        matches!(
            case.name,
            "refutable_literal_boundary" | "nested_enum_child_boundary"
        )
    }) {
        assert_eq!(case.outcome, Outcome::Runtime);
        assert!(case.stdout.is_some());
        let program = frontend::parse(case.source).unwrap();
        let signatures = frontend::signature_table(&program).unwrap();
        let mut found = 0;
        for index in 0..program.functions.len() {
            frontend::check_function_with_signatures(&program, index, &signatures).unwrap();
            let function =
                frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
            function.validate().unwrap();
            let mut patterns = Vec::new();
            for expression in &function.exprs {
                if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                    for arm in arms {
                        if matches!(arm.pattern, ir::MatchPattern::Decision { .. }) {
                            patterns.push(&arm.pattern);
                        }
                    }
                }
            }
            for statement in &function.body.statements {
                if let ir::Stmt::LetElse {
                    pattern: pattern @ ir::MatchPattern::Decision { .. },
                    ..
                } = statement
                {
                    patterns.push(pattern);
                }
            }
            for pattern in patterns {
                let ir::MatchPattern::Decision {
                    ty,
                    tests,
                    bindings,
                } = pattern
                else {
                    unreachable!()
                };
                assert!(matches!(ty, ir::Ty::Struct(_)));
                assert!(!tests.is_empty());
                if case.name == "nested_enum_child_boundary" {
                    assert_eq!(tests.len(), 1);
                    assert_eq!(tests[0].projection, vec![ir::Projection::Field(0)]);
                    let ir::PatternTestKind::EnumVariant { ty: inner, variant } = &tests[0].kind
                    else {
                        panic!("nested original enum requires a genuine tag test")
                    };
                    assert_eq!(inner.def().variants.len(), 1);
                    assert_eq!(*variant, 0);
                    assert_eq!(bindings.len(), 1);
                    assert_eq!(
                        bindings[0].projection,
                        vec![
                            ir::Projection::Field(0),
                            ir::Projection::Downcast(ir::VariantId {
                                owner: inner.def().id,
                                index: 0
                            }),
                            ir::Projection::Field(0)
                        ]
                    );
                } else {
                    assert_eq!(bindings.len(), 1);
                    assert_eq!(tests.len(), 1);
                    assert_eq!(tests[0].projection, vec![ir::Projection::Field(0)]);
                    assert!(
                        matches!(tests[0].kind,ir::PatternTestKind::Int(value) if value==ir::IntValue::i32(40))
                    );
                    assert_eq!(bindings[0].projection, vec![ir::Projection::Field(1)]);
                }
                let mut locals = std::collections::HashSet::new();
                for binding in bindings {
                    assert!(!function.params.contains(&binding.local));
                    assert!(locals.insert(binding.local));
                }
                found += 1;
            }
        }
        assert_eq!(found, 1, "{} one genuine decision plan", case.name);
        sources += 1;
    }
    assert_eq!(sources, 2);
}
