//! Nonempty fixed-array patterns with Copy-selected bindings and actual ConstantIndex plans.
//! Valid nonCopy by-value destructuring is a native Unsupported boundary, not Rust-invalid.
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
// 1239f870de9903eed5936fed6f738d4e61838a48dd1caa562ba0eb865d5fd6ca
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
            name: "let_exact",
            source: r#"fn main(){let [a,b]=[40,2];if a+b==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "match_exact",
            source: r#"fn main(){let answer=match [40,2]{[a,b]=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "prefix",
            source: r#"fn main(){let answer=match [42,7,2]{[a,..]=>a};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "suffix",
            source: r#"fn main(){let answer=match [7,40,2]{[..,a,b]=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "middle",
            source: r#"fn main(){let answer=match [40,7,2]{[a,..,b]=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "zero_omission",
            source: r#"fn main(){let answer=match [40,2]{[a,..,b]=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "allrest_noncopy_no_move",
            source: r#"struct Inner{x:i32}fn main(){let value=[Inner{x:40},Inner{x:2}];let answer=match value{[..]=>42};let next=value;if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wildcard_noncopy_no_move",
            source: r#"struct Inner{x:i32}fn main(){let value=[Inner{x:40},Inner{x:2}];let answer=match value{[_,..]=>42};let next=value;if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "copy_selected_reuse",
            source: r#"fn main(){let value=[40,7,2];let [a,..]=value;let next=value;if a+next[2]==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only",
            source: r#"fn make()->[i32;3]{println!("once");[40,7,2]}fn main(){let answer=match make(){[a,..,b]=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_noncopy_record_copyfield",
            source: r#"struct Inner{x:i32,y:i32}fn main(){let value=[Inner{x:40,y:7},Inner{x:2,y:7}];let answer=match value{[Inner{x:a,..},Inner{x:b,..}]=>a+b};let next=value;if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_captured_binder",
            source: r#"fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{match $p{[..,$n]=>$n}}};}let value=[7,42];let answer=pick!(value,n);if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_array",
            source: r#"fn main(){let answer=match [[42,7],[2,9]]{[[a,..],..]=>a};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wrong_exact_arity",
            source: r#"fn main(){let answer=match [40,2]{[a,b,c]=>a+b+c};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0527]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "too_many_rest_children",
            source: r#"fn main(){let answer=match [40,2]{[a,b,..,c]=>a+b+c};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0528]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "duplicate_binding",
            source: r#"fn main(){let answer=match [40,2]{[a,a]=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0416]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "duplicate_rest",
            source: r#"fn main(){let answer=match [40,2]{[a,..,..]=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("`..` can only be used once per slice pattern"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_scalar_domain",
            source: r#"fn main(){let answer=match 42{[a]=>a};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0529]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "valid_noncopy_selected_boundary",
            source: r#"struct Inner{x:i32}fn main(){let value=[Inner{x:40},Inner{x:2}];let [a,b]=value;if a.x+b.x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "empty_array_boundary",
            source: r#"fn main(){let value:[i32;0]=[];let []=value;println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "reference_array_boundary",
            source: r#"fn main(){let value=[40,2];let answer=match &value{[a,b]=>*a+*b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "refutable_child_boundary",
            source: r#"fn main(){let answer=match [40,2]{[40,b]=>40+b,_=>0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "guard_boundary",
            source: r#"fn main(){let answer=match [40,2]{[a,b] if a==40=>a+b,_=>0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "or_boundary",
            source: r#"fn main(){let answer=match [40,2]{[a,b]|[a,b]=>a+b};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tail_binding_boundary",
            source: r#"fn main(){let [a,tail @ ..]=[40,7,2];if a+tail[1]==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "parameter_pattern_boundary",
            source: r#"fn sum([a,b]:[i32;2])->i32{a+b}fn main(){if sum([40,2])==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "ref_binding_boundary",
            source: r#"fn main(){let value=[40,2];let [ref a,b]=value;if *a+b==42{println!("ok");}}"#,
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
            "jr-array-patterns-{}-{nonce}-{}",
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
        "ARRAY PATTERN BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=array_patterns_oracle",
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
    assert_eq!(counts, (22, 5));
    assert_eq!(partition, (18, 5, 4));
}
#[test]
fn stock_array_patterns_mapping_ownership_and_valid_unsupported_oracle_edition2024() {
    stock_all();
}
fn checked_binding_path(
    binding: &ir::PatternBinding,
    function: &ir::CheckedFunction,
) -> Vec<ir::Projection> {
    assert!(!function.params.contains(&binding.local));
    assert!(
        matches!(function.locals[binding.local.0].ty, ir::Ty::Int(_)),
        "corpus selected fields are primitive Copy integers, never whole nonCopy elements"
    );
    assert!(!binding.projection.is_empty());
    assert!(matches!(
        binding.projection[0],
        ir::Projection::ConstantIndex(_)
    ));
    assert!(
        binding.projection.iter().all(|p| matches!(
            p,
            ir::Projection::ConstantIndex(_) | ir::Projection::Field(_)
        )),
        "array binding must never gain enum Downcast or Deref"
    );
    binding.projection.clone()
}
#[test]
fn source_array_patterns_publish_exact_constant_indexes_without_omitted_bindings() {
    let mut counts = (0, 0);
    for case in cases()
        .into_iter()
        // Promoted ergonomic/ref-binding cases bind by reference through
        // Deref paths. Their plans are covered by MATCH-ERGONOMICS tests.
        .filter(|c| {
            c.outcome == Outcome::Runtime
                && !matches!(
                    c.name,
                    "refutable_child_boundary"
                        | "guard_boundary"
                        | "or_boundary"
                        | "reference_array_boundary"
                        | "ref_binding_boundary"
                )
        })
    {
        let program = frontend::parse(case.source)
            .unwrap_or_else(|e| panic!("{} parse/resolve:{e}", case.name));
        let signatures = frontend::signature_table(&program)
            .unwrap_or_else(|e| panic!("{} signatures:{e}", case.name));
        let mut actual = Vec::new();
        let mut plans = 0;
        for index in 0..program.functions.len() {
            let function = frontend::type_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} checking:{e}", case.name));
            function.validate().unwrap();
            for expression in &function.exprs {
                if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                    for arm in arms {
                        if let ir::MatchPattern::Aggregate { ty, bindings } = &arm.pattern {
                            assert!(
                                matches!(ty,ir::Ty::Array{len,..}if *len>0),
                                "array root domain must retain exact nonempty fixed-array type"
                            );
                            for binding in bindings {
                                actual.push(checked_binding_path(binding, &function));
                            }
                            plans += 1;
                        }
                    }
                }
            }
            for statement in &function.body.statements {
                if let ir::TypedStmt::LetPattern { ty, bindings, .. } = statement {
                    assert!(matches!(ty,ir::Ty::Array{len,..}if *len>0));
                    for binding in bindings {
                        actual.push(checked_binding_path(binding, &function));
                    }
                    plans += 1;
                }
            }
        }
        use ir::Projection::{ConstantIndex as I, Field as F};
        let expected = match case.name {
            "let_exact" | "match_exact" | "zero_omission" => vec![vec![I(0)], vec![I(1)]],
            "prefix" | "copy_selected_reuse" => vec![vec![I(0)]],
            "suffix" => vec![vec![I(1)], vec![I(2)]],
            "middle" | "once_only" => vec![vec![I(0)], vec![I(2)]],
            "allrest_noncopy_no_move" | "wildcard_noncopy_no_move" => vec![],
            "nested_noncopy_record_copyfield" => vec![vec![I(0), F(0)], vec![I(1), F(0)]],
            "macro_captured_binder" => vec![vec![I(1)]],
            "nested_array" => vec![vec![I(0), I(0)]],
            _ => unreachable!(),
        };
        assert_eq!(plans, 1, "{} one actual source pattern plan", case.name);
        assert_eq!(actual, expected, "{} exact physical array plan", case.name);
        counts.0 += 1;
        counts.1 += actual.len();
    }
    assert_eq!(counts, (13, 18));
}
#[test]
fn array_pattern_closed_boundaries_reject_at_their_actual_frontend_phase() {
    // Counts preserve parser, checker and resolver rejection phases separately.
    let mut counts = (0, 0, 0);
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Unsupported)
    {
        assert!(
            case.stdout.is_some(),
            "all array boundary sources are stock-valid"
        );
        let parsed = source::prepare_text(
            "array_patterns.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .and_then(|p| frontend::parse_prepared(&p));
        if case.name == "or_boundary" {
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
        } else if matches!(
            case.name,
            "guard_boundary" | "tail_binding_boundary" | "parameter_pattern_boundary"
        ) {
            let error = match parsed {
                Err(e) => e,
                Ok(_) => panic!("{} parser boundary accepted", case.name),
            };
            assert_eq!(error.kind, source::SourceErrorKind::Unsupported);
            counts.0 += 1;
        } else {
            let parsed = parsed
                .unwrap_or_else(|e| panic!("{} valid array pattern must parse:{e}", case.name));
            frontend::resolve(&parsed).unwrap_or_else(|e| {
                panic!("{} must resolve before checker boundary:{e}", case.name)
            });
            let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(e) => e,
                Ok(_) => panic!("{} must reject before publication", case.name),
            };
            let expected = match case.name {
                "valid_noncopy_selected_boundary" => {
                    "non-Copy bindings through array patterns are unsupported"
                }
                "empty_array_boundary" => "zero-length array patterns are unsupported",
                "refutable_child_boundary" => "unsupported: refutable aggregate match leaf",
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
    assert_eq!(counts, (2, 2, 0));
}
#[test]
fn native_array_patterns_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(gates, 108);
    assert_eq!(modes, (36, 36, 10, 10, 8, 8));
    eprintln!(
        "NATIVE ARRAY PATTERNS:108 cells, modes{modes:?};32 executed native images;24 stock-valid Unsupported cells are boundaries, not Rust-invalid parity or native layout/FFI proof"
    );
}

#[test]
fn retained_newly_supported_decision_witnesses_publish_real_checked_predicates() {
    let mut sources = 0;
    for case in cases()
        .into_iter()
        .filter(|case| matches!(case.name, "refutable_child_boundary"))
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
                assert!(matches!(ty, ir::Ty::Array { .. }));
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
                    assert_eq!(tests[0].projection, vec![ir::Projection::ConstantIndex(0)]);
                    assert!(
                        matches!(tests[0].kind,ir::PatternTestKind::Int(value) if value==ir::IntValue::i32(40))
                    );
                    assert_eq!(
                        bindings[0].projection,
                        vec![ir::Projection::ConstantIndex(1)]
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
