//! Genuine source if-let nodes, then-only bindings and checked conditional pattern plans.
//! Stock-valid chains and refutable enum payloads remain explicit native boundaries.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact stock JSON SHA256, baseline20 then supplement7, before native expectations:
// cf6377ec6a29630de856a3b81293f32260858496927ad6133274918441cab21a
// d9827cdbc84957571fb8936aa0a7762f5c4f9c0b9441369694e6dfb9ab917340
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
            name: "enum_then",
            source: r#"enum Choice{Some(i32),None}fn main(){let value=Choice::Some(42);let answer=if let Choice::Some(x)=value{x}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "enum_else",
            source: r#"enum Choice{Some(i32),None}fn main(){let value=Choice::None;let answer=if let Choice::Some(x)=value{x}else{42};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "omitted_else_unit",
            source: r#"enum Choice{Some(i32),None}fn main(){if let Choice::Some(x)=Choice::Some(42){if x==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "scrutinee_outer_then_shadow",
            source: r#"enum Choice{Some(i32),None}fn main(){let x=40;let answer=if let Choice::Some(x)=Choice::Some(x+2){x}else{0};if answer==42{if x==40{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "else_outer_binding",
            source: r#"enum Choice{Some(i32),None}fn main(){let x=42;let answer=if let Choice::Some(x)=Choice::None{x}else{x};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only",
            source: r#"enum Choice{Some(i32),None}fn make()->Choice{println!("once");Choice::Some(42)}fn main(){let answer=if let Choice::Some(x)=make(){x}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_move_then_only",
            source: r#"struct Inner{x:i32}enum Choice{First(Inner),Second(Inner)}fn run(value:Choice)->i32{if let Choice::First(inner)=value{inner.x}else{match value{Choice::Second(inner)=>inner.x,_=>0}}}fn main(){if run(Choice::First(Inner{x:40}))+run(Choice::Second(Inner{x:2}))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "then_diverges_preserves_else_path",
            source: r#"struct Inner{x:i32}enum Choice{First(Inner),Second(Inner)}fn run(value:Choice)->i32{if let Choice::First(inner)=value{return inner.x;}match value{Choice::Second(inner)=>inner.x,_=>0}}fn main(){if run(Choice::First(Inner{x:40}))+run(Choice::Second(Inner{x:2}))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "discriminant_only_nonmove",
            source: r#"struct Inner{x:i32}enum Choice{First(Inner),Second(Inner)}fn main(){let value=Choice::First(Inner{x:42});if let Choice::First(..)=value{}let next=value;let answer=match next{Choice::First(inner)=>inner.x,_=>0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "irrefutable_aggregate",
            source: r#"struct Pair{a:i32,b:i32}fn main(){let value=Pair{a:40,b:2};let answer=if let Pair{a,b}=value{a+b}else{0};let next=value;if answer==42{if next.a==40{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "bool_pattern",
            source: r#"fn main(){let answer=if let true=true{42}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "int_pattern",
            source: r#"fn main(){let answer=if let 7=7{42}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "binding_after_scope_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){if let Choice::Some(x)=Choice::Some(42){}let answer=x;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "binding_else_scope_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){let answer=if let Choice::Some(x)=Choice::None{x}else{x};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "scrutinee_before_binding_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){if let Choice::Some(x)=Choice::Some(x){}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "branch_type_mismatch_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){let answer=if let Choice::Some(x)=Choice::Some(42){x}else{true};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "dead_then_checked_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){if let Choice::Some(x)=Choice::None{missing();}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "possibly_moved_after_join_invalid",
            source: r#"struct Inner{x:i32}enum Choice{First(Inner),Second(Inner)}fn run(value:Choice){if let Choice::First(inner)=value{}let next=value;}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "let_chain_boundary",
            source: r#"enum Choice{Some(i32),None}fn main(){let answer=if let Choice::Some(x)=Choice::Some(42) && x==42{x}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "refutable_payload_boundary",
            source: r#"enum Choice{Some(i32),None}fn main(){let answer=if let Choice::Some(42)=Choice::Some(42){42}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_captured_binder",
            source: r#"enum Choice{Some(i32),None}fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{if let Choice::Some($n)=$p{$n}else{0}}};}let value=Choice::Some(42);let answer=pick!(value,n);if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_marked_binding",
            source: r#"enum Choice{Some(i32),None}fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{if let Choice::Some(x)=$p{if x==40{$n+2}else{0}}else{0}}};}let x=40;let value=Choice::Some(40);let answer=pick!(value,x);if answer==42{if x==40{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "else_if",
            source: r#"enum Choice{Some(i32),None}fn main(){let answer=if let Choice::Some(x)=Choice::None{x}else if true{42}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "else_if_let",
            source: r#"enum Choice{Some(i32),None}fn main(){let answer=if let Choice::Some(x)=Choice::None{x}else if let Choice::Some(y)=Choice::Some(42){y}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_existing",
            source: r#"enum Choice<T>{Some(T),None}fn take(value:Choice<i32>)->i32{if let Choice::Some(x)=value{x}else{0}}fn main(){if take(Choice::Some(42))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "imported_variant",
            source: r#"mod values{pub enum Choice{Some(i32),None}}use values::Choice::Some as Present;fn main(){let answer=if let Present(x)=Present(42){x}else{0};if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "dead_else_checked_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){let answer=if let Choice::Some(x)=Choice::Some(42){x}else{missing()};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
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
            "jr-if-let-{}-{nonce}-{}",
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
        "IF-LET BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=if_let_oracle",
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
    assert_eq!(counts, (20, 7));
    assert_eq!(partition, (19, 7, 1));
}
#[test]
fn stock_if_let_mapping_ownership_and_valid_unsupported_oracle_edition2024() {
    stock_all();
}

fn direct_if_let(block: &frontend::Block) -> &frontend::Expr {
    if let Some(value) = block.tail.as_deref()
        && matches!(value, frontend::Expr::IfLet { .. })
    {
        return value;
    }
    for statement in &block.statements {
        let value = match statement {
            frontend::Stmt::Let { value, .. }
            | frontend::Stmt::Expr(value)
            | frontend::Stmt::ImplicitExpr(value) => value,
            _ => continue,
        };
        if matches!(value, frontend::Expr::IfLet { .. }) {
            return value;
        }
    }
    panic!("expected actual source if-let, not fabricated Match");
}
#[test]
fn source_if_let_retains_absent_else_and_actual_nested_expression_shapes() {
    for name in ["enum_then", "omitted_else_unit", "else_if", "else_if_let"] {
        let case = cases().into_iter().find(|c| c.name == name).unwrap();
        let prepared =
            source::prepare_text("if_let.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let function = &parsed.program.functions[0];
        let frontend::Expr::IfLet {
            pattern,
            scrutinee,
            then_block,
            else_branch,
        } = direct_if_let(&function.body)
        else {
            unreachable!()
        };
        assert!(matches!(
            pattern.as_ref(),
            frontend::Pattern::Variant { .. }
        ));
        assert!(!matches!(
            scrutinee.as_ref(),
            frontend::Expr::IfLet { .. } | frontend::Expr::Match { .. }
        ));
        assert!(!then_block.statements.is_empty() || then_block.tail.is_some());
        match name {
            "omitted_else_unit" => assert!(
                else_branch.is_none(),
                "absence must not become an empty source block"
            ),
            "enum_then" => assert!(matches!(
                else_branch.as_deref(),
                Some(frontend::Expr::Block(_))
            )),
            "else_if" => assert!(matches!(
                else_branch.as_deref(),
                Some(frontend::Expr::If { .. })
            )),
            "else_if_let" => assert!(matches!(
                else_branch.as_deref(),
                Some(frontend::Expr::IfLet { .. })
            )),
            _ => unreachable!(),
        }
    }
}
#[test]
fn real_conditional_patterns_type_all_functions_and_preserve_checked_enum_ownership() {
    let mut counts = (0, 0);
    for case in cases().into_iter().filter(|c| {
        c.outcome == Outcome::Runtime && !matches!(c.name, "refutable_payload_boundary")
    }) {
        let program = frontend::parse(case.source)
            .unwrap_or_else(|e| panic!("{} parse/resolve:{e}", case.name));
        if case.name.starts_with("macro_") {
            assert!(
                program.functions.iter().any(|f| f.lexical_scope.is_some()),
                "{} requires actual scoped macro source",
                case.name
            );
        }
        let signatures = frontend::signature_table(&program)
            .unwrap_or_else(|e| panic!("{} signatures:{e}", case.name));
        let mut plans = 0;
        for index in 0..program.functions.len() {
            let function = frontend::type_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} checker:{e}", case.name));
            function.validate().unwrap();
            for expression in &function.exprs {
                if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                    assert_eq!(
                        arms.len(),
                        2,
                        "{} conditional or explicit corpus match has two real branches",
                        case.name
                    );
                    assert!(
                        matches!(arms[1].pattern, ir::MatchPattern::Wildcard),
                        "{} semantic fallback",
                        case.name
                    );
                    if let ir::MatchPattern::EnumVariant {
                        ty,
                        variant,
                        bindings,
                    } = &arms[0].pattern
                    {
                        assert!(*variant < ty.def().variants.len());
                        for binding in bindings {
                            assert!(!function.params.contains(&binding.local));
                            assert!(
                                matches!(binding.projection.first(), Some(ir::Projection::Downcast(id)) if id.owner == ty.def().id && id.index == *variant)
                            );
                            assert!(
                                binding
                                    .projection
                                    .iter()
                                    .skip(1)
                                    .all(|p| matches!(p, ir::Projection::Field(_)))
                            );
                        }
                    }
                    plans += 1;
                }
            }
        }
        let expected = match case.name {
            "noncopy_move_then_only"
            | "then_diverges_preserves_else_path"
            | "discriminant_only_nonmove"
            | "else_if_let" => 2,
            _ => 1,
        };
        assert_eq!(plans, expected, "{} once-only conditional plans", case.name);
        counts.0 += 1;
        counts.1 += plans;
    }
    assert_eq!(counts, (18, 22));
}
#[test]
fn legal_closed_if_let_cases_reject_at_their_actual_frontend_phase() {
    let mut counts = (0, 0);
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Unsupported)
    {
        assert!(case.stdout.is_some(), "closed witnesses are stock-valid");
        let prepared =
            source::prepare_text("if_let.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed = frontend::parse_prepared(&prepared);
        if case.name == "let_chain_boundary" {
            let error = match parsed {
                Err(e) => e,
                Ok(_) => panic!("chain must not publish bindings"),
            };
            assert_eq!(error.kind, source::SourceErrorKind::Unsupported);
            assert!(error.to_string().contains("if let chains are unsupported"));
            counts.0 += 1;
        } else {
            let parsed = parsed.unwrap();
            frontend::resolve(&parsed).unwrap();
            let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(e) => e,
                Ok(_) => panic!("refutable payload must not publish executable"),
            };
            assert!(
                failure
                    .to_string()
                    .contains("nested refutable literal patterns are unsupported"),
                "{}:{failure}",
                case.name
            );
            counts.1 += 1;
        }
    }
    assert_eq!(counts, (1, 0));
}
#[test]
fn native_if_let_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(modes, (38, 38, 14, 14, 2, 2));
    eprintln!(
        "NATIVE IF-LET:108 cells, modes{modes:?};38 executed native images;4 stock-valid Unsupported cells are boundaries, not Rust-invalid parity or full Rust support"
    );
}

#[test]
fn retained_newly_supported_decision_witnesses_publish_real_checked_predicates() {
    let mut sources = 0;
    for case in cases()
        .into_iter()
        .filter(|case| matches!(case.name, "refutable_payload_boundary"))
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
                    assert_eq!(bindings.len(), 0);
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
                        matches!(tests[1].kind,ir::PatternTestKind::Int(value) if value==ir::IntValue::i32(42))
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
