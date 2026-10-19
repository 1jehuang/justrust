//! Genuine enum tuple-payload rest: actual variant tags, Downcast and physical Field indexes.
//! Multi-variant let is stock-invalid/native-Unsupported, not ordinary semantic parity.
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
// d9d47e221ce456c99d51d8f995c37734469c495f808edaca3e114b645df62c14
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Invalid,
    Unsupported,
    InvalidUnsupported,
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
            name: "middle",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let result=match Item::Only(40,7,2){Item::Only(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "leading",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let result=match Item::Only(7,40,2){Item::Only(..,a,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "trailing",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let result=match Item::Only(40,2,7){Item::Only(a,b,..)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "zero_omission",
            source: r#"enum Item{Only(i32,i32)}fn main(){let result=match Item::Only(40,2){Item::Only(a,..,b)=>a+b};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "all_rest",
            source: r#"enum Item{Only(i32,i32)}fn main(){let result=match Item::Only(40,2){Item::Only(..)=>42};if result==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "coverage_two_tags",
            source: r#"enum Item{First(i32,i32,i32),Second(i32,i32)}fn run(value:Item)->i32{match value{Item::First(a,..,b)=>a+b,Item::Second(..,b)=>b}}fn main(){if run(Item::First(40,7,2))+run(Item::Second(7,42))==84{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wildcard_default",
            source: r#"enum Item{First(i32,i32,i32),Second(i32)}fn run(value:Item)->i32{match value{Item::First(a,..,b)=>a+b,_=>42}}fn main(){if run(Item::First(40,7,2))+run(Item::Second(7))==84{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "single_variant_let",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let Item::Only(a,..,b)=Item::Only(40,7,2);if a+b==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "scalar_copy_reuse",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let value=Item::Only(40,7,2);let answer=match value{Item::Only(a,..,b)=>a+b};let next=value;let Item::Only(..,b)=next;if answer+b==44{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "discriminant_only_nonmove",
            source: r#"struct Inner{x:i32}enum Item{First(Inner,i32),Second(Inner)}fn main(){let value=Item::First(Inner{x:40},2);let answer=match value{Item::First(..)=>42,Item::Second(..)=>0};let next=value;if answer==42{let result=match next{Item::First(inner,..)=>inner.x,_=>0};if result==40{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only",
            source: r#"enum Item{Only(i32,i32,i32)}fn make()->Item{println!("once");Item::Only(40,7,2)}fn main(){let answer=match make(){Item::Only(a,..,b)=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "selected_noncopy_return",
            source: r#"struct Inner{x:i32}enum Item{Only(Inner,i32)}fn main(){let value=Item::Only(Inner{x:42},2);let inner=match value{Item::Only(inner,..)=>inner};if inner.x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "omitted_noncopy_reuse",
            source: r#"struct Inner{x:i32}enum Item{Only(i32,Inner,i32)}fn main(){let value=Item::Only(40,Inner{x:7},2);let answer=match value{Item::Only(a,..,b)=>a+b};let next=value;let Item::Only(_,inner,..)=next;if answer==42{if inner.x==7{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "imported_variant",
            source: r#"mod m{pub enum Item{Only(i32,i32,i32)}}use m::Item::Only as Alias;fn main(){let answer=match Alias(40,7,2){Alias(a,..,b)=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "existing_generic_enum",
            source: r#"enum Item<T>{Only(T,i32,i32)}fn main(){let value:Item<i32>=Item::Only(40,7,2);let answer=match value{Item::Only(a,..,b)=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wrong_arity",
            source: r#"enum Item{Only(i32,i32)}fn main(){let answer=match Item::Only(40,2){Item::Only(a,b,..,c)=>a+b+c};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0023]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_nominal",
            source: r#"enum A{Only(i32,i32)}enum B{Only(i32,i32)}fn main(){let answer=match B::Only(40,2){A::Only(a,..)=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "selected_noncopy_move",
            source: r#"struct Inner{x:i32}enum Item{Only(Inner,i32)}fn main(){let value=Item::Only(Inner{x:40},2);let inner=match value{Item::Only(inner,..)=>inner};let next=value;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_coverage",
            source: r#"enum Item{First(i32,i32),Second(i32)}fn main(){let answer=match Item::First(40,2){Item::First(a,..)=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0004]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "multivariant_let",
            source: r#"enum Item{First(i32,i32),Second(i32)}fn main(){let Item::First(a,..)=Item::First(42,2);if a==42{println!("ok");}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0005]"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "refutable_child",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let answer=match Item::Only(40,7,2){Item::Only(40,..,b)=>40+b,_=>0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "guard",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let answer=match Item::Only(40,7,2){Item::Only(a,..,b) if a==40=>a+b,_=>0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "refbinding",
            source: r#"enum Item{Only(i32,i32,i32)}fn main(){let value=Item::Only(40,7,2);let answer=match value{Item::Only(ref a,..,b)=>*a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "zero_payload_tuple_variant",
            source: r#"enum Item{Zero()}fn main(){let result=match Item::Zero(){Item::Zero(..)=>42};if result==42{println!("ok");}}"#,
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
            "jr-enum-rest-patterns-{}-{nonce}-{}",
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
        "ENUM REST BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    let mut partition = (0, 0, 0, 0);
    for case in cases() {
        match case.outcome {
            Outcome::Runtime => partition.0 += 1,
            Outcome::Invalid => partition.1 += 1,
            Outcome::Unsupported => partition.2 += 1,
            Outcome::InvalidUnsupported => partition.3 += 1,
        }
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=enum_rest_oracle",
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
    assert_eq!(counts, (19, 5));
    assert_eq!(partition, (18, 4, 1, 1));
}
#[test]
fn stock_enum_rest_payload_mapping_coverage_ownership_oracle_edition2024() {
    stock_all();
}
fn check_binding(
    binding: &ir::PatternBinding,
    ty: &ir::EnumTy,
    variant: usize,
    function: &ir::CheckedFunction,
) -> usize {
    assert!(!function.params.contains(&binding.local));
    assert_eq!(
        binding.projection.len(),
        2,
        "direct payload binder needs genuine Downcast then Field"
    );
    assert_eq!(
        binding.projection[0],
        ir::Projection::Downcast(ir::VariantId {
            owner: ty.def().id,
            index: variant
        })
    );
    let ir::Projection::Field(index) = binding.projection[1] else {
        panic!("enum binding missing physical payload Field")
    };
    assert!(index < ty.def().variants[variant].fields.len());
    index
}
#[test]
fn enum_rest_ir_retains_variant_authority_actual_downcasts_and_payload_indexes() {
    let mut counts = (0, 0);
    for case in cases().into_iter().filter(|c| {
        c.outcome == Outcome::Runtime && !matches!(c.name, "refutable_child" | "refbinding")
    }) {
        let program = frontend::parse(case.source)
            .unwrap_or_else(|e| panic!("{} parse/resolve:{e}", case.name));
        let signatures = frontend::signature_table(&program)
            .unwrap_or_else(|e| panic!("{} signatures:{e}", case.name));
        let mut selected = Vec::new();
        let mut enum_arms = 0;
        let mut lets = 0;
        for index in 0..program.functions.len() {
            let function = frontend::type_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} checking:{e}", case.name));
            function.validate().unwrap();
            for expression in &function.exprs {
                if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                    for arm in arms {
                        match &arm.pattern {
                            ir::MatchPattern::EnumVariant {
                                ty,
                                variant,
                                bindings,
                            } => {
                                assert_eq!(
                                    ty.def().variants[*variant].shape,
                                    ir::VariantShape::Tuple
                                );
                                for binding in bindings {
                                    selected.push((
                                        *variant,
                                        check_binding(binding, ty, *variant, &function),
                                    ));
                                }
                                enum_arms += 1;
                            }
                            ir::MatchPattern::Aggregate { .. } => panic!(
                                "{} enum rest must never lower as aggregate catchall",
                                case.name
                            ),
                            _ => {}
                        }
                    }
                }
            }
            for statement in &function.body.statements {
                if let ir::TypedStmt::LetPattern {
                    ty: ir::Ty::Enum(ty),
                    bindings,
                    ..
                } = statement
                {
                    assert_eq!(ty.def().variants.len(), 1);
                    for binding in bindings {
                        check_binding(binding, ty, 0, &function);
                    }
                    lets += 1;
                }
            }
        }
        let expected: Vec<(usize, usize)> = match case.name {
            "middle"
            | "once_only"
            | "scalar_copy_reuse"
            | "omitted_noncopy_reuse"
            | "imported_variant"
            | "existing_generic_enum"
            | "wildcard_default" => vec![(0, 0), (0, 2)],
            "leading" => vec![(0, 1), (0, 2)],
            "trailing" | "zero_omission" => vec![(0, 0), (0, 1)],
            "coverage_two_tags" => vec![(0, 0), (0, 2), (1, 1)],
            "selected_noncopy_return" | "discriminant_only_nonmove" => vec![(0, 0)],
            "all_rest" | "zero_payload_tuple_variant" | "single_variant_let" => vec![],
            _ => unreachable!(),
        };
        assert_eq!(
            selected, expected,
            "{} physical prefix/suffix projection mismatch",
            case.name
        );
        if case.name == "single_variant_let" {
            assert_eq!(enum_arms, 0);
            assert_eq!(lets, 1);
        } else {
            assert!(enum_arms >= 1, "{} actual enum match absent", case.name);
        }
        counts.0 += 1;
        counts.1 += 1;
    }
    assert_eq!(counts, (16, 16));
}
#[test]
fn enum_rest_closed_patterns_reject_at_their_actual_frontend_phase() {
    let mut counts = (0, 0, 0);
    for case in cases().into_iter().filter(|c| {
        matches!(
            c.outcome,
            Outcome::Unsupported | Outcome::InvalidUnsupported
        )
    }) {
        let parsed = source::prepare_text(
            "enum_rest.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .and_then(|p| frontend::parse_prepared(&p));
        if matches!(case.name, "guard") {
            let error = match parsed {
                Err(e) => e,
                Ok(_) => panic!("{} parser boundary accepted", case.name),
            };
            assert_eq!(error.kind, source::SourceErrorKind::Unsupported);
            counts.0 += 1;
        } else {
            let parsed = parsed
                .unwrap_or_else(|e| panic!("{} valid pattern grammar must parse:{e}", case.name));
            frontend::resolve(&parsed)
                .unwrap_or_else(|e| panic!("{} must resolve before checking:{e}", case.name));
            let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(e) => e,
                Ok(_) => panic!("{} must reject before publication", case.name),
            };
            assert!(
                failure.to_string().contains("unsupported"),
                "{}:{failure}",
                case.name
            );
            if case.outcome == Outcome::InvalidUnsupported {
                assert_eq!(case.stock_diagnostic, Some("error[E0005]"));
                assert!(case.stdout.is_none());
                counts.2 += 1;
            } else {
                counts.1 += 1;
            }
        }
    }
    assert_eq!(counts, (1, 0, 1));
}
#[test]
fn native_enum_rest_workers1_8_check_build_no_compiler_fallback() {
    // jr-rust --profile emits telemetry, not optimization. The coordinator runs
    // this same target under debug and release BOOTSTRAP compiler profiles.
    // Neither run asserts a nonexistent optimized native code-generation mode.
    stock_all();
    let mut gates = 0;
    let mut modes = (0, 0, 0, 0, 0, 0, 0, 0);
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
                    if matches!(
                        case.outcome,
                        Outcome::Unsupported | Outcome::InvalidUnsupported
                    ) {
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
                    (Outcome::InvalidUnsupported, true) => modes.6 += 1,
                    (Outcome::InvalidUnsupported, false) => modes.7 += 1,
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 96);
    assert_eq!(modes, (36, 36, 8, 8, 2, 2, 2, 2));
    eprintln!(
        "NATIVE ENUM REST:96 cells, modes{modes:?};36 executed native images;4 stock-valid Unsupported cells plus4 stock-invalid/native-Unsupported cells are boundaries, not Rust-invalid parity or native layout/FFI proof"
    );
}

#[test]
fn retained_newly_supported_decision_witnesses_publish_real_checked_predicates() {
    let mut sources = 0;
    for case in cases()
        .into_iter()
        .filter(|case| matches!(case.name, "refutable_child"))
    {
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
                if let ir::TypedStmt::LetElse {
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
                assert!(matches!(ty, ir::Ty::Enum(_)));
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
                    assert_eq!(tests.len(), 2);
                    assert!(tests[0].projection.is_empty());
                    let ir::PatternTestKind::EnumVariant {
                        ty: enumeration,
                        variant,
                    } = &tests[0].kind
                    else {
                        panic!("root tag test missing")
                    };
                    assert_eq!(ty, &ir::Ty::Enum(enumeration.clone()));
                    let downcast = ir::Projection::Downcast(ir::VariantId {
                        owner: enumeration.def().id,
                        index: *variant,
                    });
                    assert_eq!(
                        tests[1].projection,
                        vec![downcast, ir::Projection::Field(0)]
                    );
                    assert!(
                        matches!(tests[1].kind,ir::PatternTestKind::Int(value) if value==ir::IntValue::i32(40))
                    );
                    assert_eq!(
                        bindings[0].projection,
                        vec![downcast, ir::Projection::Field(2)]
                    );
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
    assert_eq!(sources, 1);
}
