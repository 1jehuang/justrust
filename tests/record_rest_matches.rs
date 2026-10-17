//! Named-record rest matches with checked selected fields and omitted-field ownership.
//! Unit rest and refutable/guard/or/ref patterns remain explicit boundaries.
//! Existing top-level enum-record rest support is retained as a separate control.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact pre-native27-source stock JSON SHA256:
// 3e894be5c8dfffb70446faeeee744b495edd0610f548c954e7fda927ba857c19
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
            name: "selected_field",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:42,b:2};let answer=match value{Pair{a,..}=>a};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "omitted_private_cross_module",
            source: r#"mod m{pub struct Pair{pub a:i32,b:i32}pub fn make()->Pair{Pair{a:42,b:2}}}fn main(){let answer=match m::make(){m::Pair{a,..}=>a};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "omitted_noncopy_reused",
            source: r#"struct Inner{x:i32}struct Pair{inner:Inner,n:i32}fn main(){let value=Pair{inner:Inner{x:40},n:2};let n=match value{Pair{n,..}=>n};let next=value;if next.inner.x+n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "selected_noncopy_return",
            source: r#"struct Inner{x:i32}struct Pair{inner:Inner,n:i32}fn main(){let value=Pair{inner:Inner{x:42},n:2};let inner=match value{Pair{inner,..}=>inner};if inner.x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "all_rest_no_move",
            source: r#"struct Inner{x:i32}struct Pair{inner:Inner,n:i32}fn main(){let value=Pair{inner:Inner{x:40},n:2};let n=match value{Pair{..}=>2};let next=value;if next.inner.x+n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_record_rest",
            source: r#"struct Inner{x:i32,y:i32}struct Outer{inner:Inner,n:i32}fn main(){let value=Outer{inner:Inner{x:42,y:1},n:2};let answer=match value{Outer{inner:Inner{x,..},..}=>x};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "renamed_reordered_fields",
            source: r#"struct Data{a:i32,b:i32,c:i32}fn main(){let value=Data{a:40,b:2,c:7};let answer=match value{Data{b:y,a:x,..}=>x+y};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "selected_wildcard_no_move",
            source: r#"struct Inner{x:i32}struct Pair{inner:Inner,n:i32}fn main(){let value=Pair{inner:Inner{x:40},n:2};let n=match value{Pair{inner:_,..}=>2};let next=value;if next.inner.x+n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only_scrutinee",
            source: r#"struct Pair{a:i32,b:i32}fn make()->Pair{println!("once");Pair{a:42,b:2}}fn main(){let answer=match make(){Pair{a,..}=>a};if answer==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_tuple_without_tuple_rest",
            source: r#"struct Pair{pair:(i32,i32),n:i32}fn main(){let value=Pair{pair:(40,2),n:7};let answer=match value{Pair{pair:(a,b),..}=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "selected_private_invalid",
            source: r#"mod m{pub struct Pair{pub a:i32,b:i32}pub fn make()->Pair{Pair{a:40,b:2}}}fn main(){let answer=match m::make(){m::Pair{b,..}=>b};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0451]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "selected_noncopy_moves",
            source: r#"struct Inner{x:i32}struct Pair{inner:Inner,n:i32}fn main(){let value=Pair{inner:Inner{x:40},n:2};let inner=match value{Pair{inner,..}=>inner};let next=value;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_nominal",
            source: r#"struct A{x:i32,y:i32}struct B{x:i32,y:i32}fn main(){let value=B{x:42,y:1};let answer=match value{A{x,..}=>x};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "duplicate_field",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{a:x,a:y,..}=>x+y};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0025]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_without_rest",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{a}=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0027]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unknown_field",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{z,..}=>z};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0026]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "duplicate_binding",
            source: r#"struct Pair{a:i32,b:i32,c:i32}fn main(){let value=Pair{a:40,b:2,c:7};let answer=match value{Pair{a:x,b:x,..}=>x};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0416]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "malformed_rest",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{a,..,..}=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("expected `}`, found `,`"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "tuple_struct_rest_boundary",
            source: r#"struct Pair(i32,i32);fn main(){let answer=match Pair(42,2){Pair(a,..)=>a};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "structural_tuple_rest_boundary",
            source: r#"fn main(){let answer=match (42,2){(a,..)=>a};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_rest_boundary",
            source: r#"struct Marker;fn main(){let answer=match Marker{Marker{..}=>42};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "enum_rest_boundary",
            source: r#"enum Item{Only{x:i32,y:i32}}fn main(){let value=Item::Only{x:42,y:2};let answer=match value{Item::Only{x,..}=>x};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "refutable_leaf_boundary",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{a:40,b,..}=>40+b,_=>0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "guard_boundary",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{a,b,..} if a==40=>a+b,_=>0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "or_boundary",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{a,b,..}|Pair{a,b,..}=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "ref_binding_boundary",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=match value{Pair{a:ref a,b,..}=>*a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "macro_captured_binding_rest",
            source: r#"struct R{a:i32,b:i32}fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{match $p{R{a:$n,..}=>$n}}};}let value=R{a:40,b:2};let answer=pick!(value,n);if answer+value.b==42{println!("ok");}}"#,
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
            "jr-record-rest-matches-{}-{nonce}-{}",
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
        "RECORD REST MATCH BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=record_rest_matches_oracle",
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
    assert_eq!(counts, (19, 8));
    assert_eq!(partition, (14, 8, 5));
}
#[test]
fn stock_record_rest_privacy_ownership_oracle_edition2024() {
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
fn source_record_rest_ir_retains_only_selected_plain_field_bindings() {
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
        if case.name == "enum_rest_boundary" {
            assert_eq!(
                aggregate_arms, 0,
                "existing enum rest is not a record Aggregate"
            );
            let enums = function
                .exprs
                .iter()
                .filter_map(|expression| {
                    if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                        Some(arms)
                    } else {
                        None
                    }
                })
                .flatten()
                .filter(|arm| matches!(arm.pattern, ir::MatchPattern::EnumVariant { .. }))
                .count();
            assert_eq!(
                enums, 1,
                "existing enum rest must retain its actual Enum IR"
            );
        } else {
            assert_eq!(
                aggregate_arms, 1,
                "{} expected one genuine record aggregate arm",
                case.name
            );
        }
        let expected = match case.name {
            "all_rest_no_move" | "selected_wildcard_no_move" | "enum_rest_boundary" => 0,
            "renamed_reordered_fields" | "nested_tuple_without_tuple_rest" => 2,
            _ => 1,
        };
        assert_eq!(
            seen.len(),
            expected,
            "{} selected bindings must exclude omitted fields",
            case.name
        );
        counts.0 += 1;
        counts.1 += seen.len();
    }
    assert_eq!(counts, (14, 13));
}

#[test]
fn record_rest_closed_domains_reject_in_their_actual_frontend_phase() {
    let mut counts = (0, 0);
    for case in cases()
        .into_iter()
        .filter(|case| case.outcome == Outcome::Unsupported)
    {
        let parsed = source::prepare_text(
            "record_rest.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .and_then(|prepared| frontend::parse_prepared(&prepared));
        if matches!(
            case.name,
            "guard_boundary" | "or_boundary" | "ref_binding_boundary"
        ) {
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
                "refutable_leaf_boundary" => "unsupported: refutable aggregate match leaf",
                "unit_rest_boundary" => "unsupported: unit struct pattern fields or rest",
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
    assert_eq!(counts, (3, 2));
}
#[test]
fn native_record_rest_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(gates, 108);
    assert_eq!(modes, (28, 28, 16, 16, 10, 10));
    eprintln!(
        "NATIVE RECORD REST MATCHES:108 cells, modes{modes:?};28 executed native images;20 Unsupported cells are boundaries, not Rust-invalid parity or native layout/FFI proof"
    );
}
