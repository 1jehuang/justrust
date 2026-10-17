//! Genuine while-let source statements, iteration-only bindings and checked Loop/Match plans.
//! Source Unit obligations are independent of the lowered loop exit completion.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact baseline6 stock JSON SHA256:
// 4a452dd1b39af975da7735e3bb65b0ae377d9bc552f89fc235e94efdae9d6f6a
// Exact full29 stock JSON SHA256, measured before native expectations:
// 0af2d3a27fb056c5b491d337be96b2f581a41ffb45ffd00d1b803c17c438181b
// Four appended stock completion controls, original29 unchanged:
// 6653c903260b48b2c7fa012d80b21f0c025392629e70d2067482babe3a272679
// 9350ed6246e94f9eeeb004eb66e978c2fffc5bd97e4f4939f279b45552a8f2f2
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
            name: "once_per_test_continue_false",
            source: r#"enum Choice{Some(i32),None}fn step(n:i32)->Choice{println!("step");if n<3{Choice::Some(n)}else{Choice::None}}fn main(){let mut n=0;let mut sum=0;while let Choice::Some(x)=step(n){n=n+1;if x==1{continue;}sum=sum+x;}if sum==2{if n==3{println!("ok");}}}"#,
            stdout: Some("step\nstep\nstep\nstep\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "shadow_restored",
            source: r#"enum Choice{Some(i32),None}fn main(){let x=42;let mut value=Choice::Some(7);while let Choice::Some(x)=value{if x==7{value=Choice::None;}}if x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_success_return_false_reuse",
            source: r#"struct Inner{x:i32}enum Choice{First(Inner),Second(Inner)}fn run(value:Choice)->i32{while let Choice::First(inner)=value{return inner.x;}match value{Choice::Second(inner)=>inner.x,_=>0}}fn main(){if run(Choice::First(Inner{x:40}))+run(Choice::Second(Inner{x:2}))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "labelled_break_continue",
            source: r#"enum Choice{Some(i32),None}fn main(){let mut n=0;'outer: while let Choice::Some(x)=Choice::Some(n){n=n+1;if x==0{continue 'outer;}break 'outer;}if n==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "moved_loopback_invalid",
            source: r#"struct Inner{x:i32}enum Choice{Some(Inner),None}fn consume(value:Inner){}fn run(value:Choice){while let Choice::Some(inner)=value{consume(inner);}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "scope_escape_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){while let Choice::Some(x)=Choice::None{}let y=x;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "initial_false",
            source: r#"enum Choice{Some(i32),None}fn main(){let mut n=0;while let Choice::Some(x)=Choice::None{n=n+x;}if n==0{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_captured_binder",
            source: r#"enum Choice{Some(i32),None}fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{let mut answer=0;while let Choice::Some($n)=$p{answer=$n;break;}answer}};}let value=Choice::Some(42);let answer=pick!(value,n);if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "imported_variant",
            source: r#"mod values{pub enum Choice{Some(i32),None}}use values::Choice::Some as Present;fn main(){let mut answer=0;while let Present(x)=Present(42){answer=x;break;}if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_labels",
            source: r#"enum Choice{Some(i32),None}fn step(n:i32)->Choice{if n<2{Choice::Some(n)}else{Choice::None}}fn main(){let mut n=0;'outer: while let Choice::Some(x)=step(n){while let true=true{n=n+1;continue 'outer;}}if n==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "condition_outer_binding",
            source: r#"enum Choice{Some(i32),None}fn main(){let x=40;let mut answer=0;while let Choice::Some(x)=Choice::Some(x+2){answer=x;break;}if answer==42{if x==40{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_pattern",
            source: r#"fn main(){while let ()=(){break;}println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "never_condition",
            source: r#"fn value()->i32{while let true=(loop{}){}}fn main(){println!("ok");}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "dead_following_value",
            source: r#"fn value()->i32{while let _=1{}42}fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_reinitialize",
            source: r#"struct Inner{x:i32}enum Choice{Some(Inner),None}fn main(){let mut value=Choice::Some(Inner{x:42});let mut answer=0;while let Choice::Some(inner)=value{answer=inner.x;value=Choice::None;}let next=value;if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "sparse_suffix",
            source: r#"enum Choice{Some(i32,i32,i32),None}fn main(){let mut answer=0;while let Choice::Some(a,..,z)=Choice::Some(40,7,2){answer=a+z;break;}if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_marked_binding",
            source: r#"enum Choice{Some(i32),None}fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{let mut answer=0;while let Choice::Some(x)=$p{answer=x+$n;break;}answer}};}let x=2;let value=Choice::Some(40);let answer=pick!(value,x);if answer==42{if x==2{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "scrutinee_before_binding_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){while let Choice::Some(x)=Choice::Some(x){}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "dead_body_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){while let Choice::Some(x)=Choice::None{missing();}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "body_value_invalid",
            source: r#"fn main(){while let true=false{42}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "valued_break_invalid",
            source: r#"fn main(){while let true=true{break 42;}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0571]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_return_invalid",
            source: r#"fn value()->i32{while let _=1{}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "never_dead_body_invalid",
            source: r#"fn value()->i32{while let true=(loop{}){missing();}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "let_chain_boundary",
            source: r#"enum Choice{Some(i32),None}fn main(){while let Choice::Some(x)=Choice::Some(42) && x==42{println!("ok");break;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "refutable_payload_boundary",
            source: r#"enum Choice{Some(i32),None}fn main(){while let Choice::Some(42)=Choice::Some(42){println!("ok");break;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "reference_binding_boundary",
            source: r#"enum Choice{Some(i32),None}fn main(){let value=Choice::Some(42);while let Choice::Some(ref x)=value{if *x==42{println!("ok");}break;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "or_boundary",
            source: r#"enum Choice{Some(i32),Other(i32)}fn main(){while let Choice::Some(x)|Choice::Other(x)=Choice::Some(42){if x==42{println!("ok");}break;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "noncopy_array_binding_boundary",
            source: r#"struct Inner{x:i32}fn main(){let value=[Inner{x:42}];while let [a]=value{if a.x==42{println!("ok");}break;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "never_condition_unit_return",
            source: r#"fn value(){while let true=(loop{}){}}fn main(){println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "return_before_tail_while_invalid",
            source: r#"fn value()->i32{return 7;while let true=(loop{}){}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "return_before_semicolon",
            source: r#"fn value()->i32{return 7;while let true=(loop{}){};}fn main(){}"#,
            stdout: Some(""),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "return_before_following_let",
            source: r#"fn value()->i32{return 7;while let true=(loop{}){}let x=1;}fn main(){}"#,
            stdout: Some(""),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "return_before_following_return",
            source: r#"fn value()->i32{return 7;while let true=(loop{}){}return 8;}fn main(){}"#,
            stdout: Some(""),
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
            "jr-while-let-{}-{nonce}-{}",
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
fn runtime(path: &Path, expected: &str, scratch: &Scratch, empty_main: bool) {
    assert!(
        !expected.is_empty() || empty_main,
        "runtime oracle cannot be empty except for exact measured completion controls"
    );
    let image = fs::read(path).unwrap();
    assert!(image.len() >= 64);
    assert_eq!(&image[..6], b"\x7fELF\x02\x01");
    let result = capture(&mut Command::new(path), scratch, 5);
    assert_eq!(result.code, 0, "runtime: {result:?}");
    assert_eq!(result.stdout, expected.as_bytes());
    assert!(result.stderr.is_empty(), "{result:?}");
}
fn empty_main_control(case: &Case) -> bool {
    let allowed = matches!(
        case.name,
        "return_before_semicolon"
            | "return_before_following_let"
            | "return_before_following_return"
    );
    if allowed {
        assert_eq!(case.stdout, Some(""));
        assert!(case.source.ends_with("fn main(){}"));
    }
    allowed
}
fn stock_all() {
    let scratch = Scratch::new();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = capture(Command::new(&rustc).arg("-vV"), &scratch, 10);
    assert_eq!(version.code, 0);
    eprintln!(
        "WHILE-LET BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=while_let_oracle",
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
            runtime(&image, expected, &scratch, empty_main_control(&case));
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
    assert_eq!(counts, (23, 10));
    assert_eq!(partition, (18, 10, 5));
}
#[test]
fn stock_while_let_mapping_ownership_and_valid_unsupported_oracle_edition2024() {
    stock_all();
}

#[test]
fn source_while_let_is_a_genuine_statement_with_actual_label_and_boxed_children() {
    for name in [
        "once_per_test_continue_false",
        "labelled_break_continue",
        "condition_outer_binding",
        "unit_pattern",
        "return_before_semicolon",
    ] {
        let case = cases().into_iter().find(|c| c.name == name).unwrap();
        let prepared = source::prepare_text(
            "while_let.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let statement = parsed
            .program
            .functions
            .iter()
            .flat_map(|f| &f.body.statements)
            .find(|s| matches!(s, frontend::Stmt::WhileLet { .. }))
            .expect("actual source WhileLet, never fabricated source Loop/Match");
        let frontend::Stmt::WhileLet {
            label,
            pattern,
            scrutinee,
            body,
            has_semicolon,
        } = statement
        else {
            unreachable!()
        };
        assert_eq!(label.is_some(), name == "labelled_break_continue");
        assert_eq!(
            *has_semicolon,
            name == "return_before_semicolon",
            "retain the actual written semicolon, not inferred source completion"
        );
        assert!(!matches!(
            scrutinee.as_ref(),
            frontend::Expr::Match { .. } | frontend::Expr::IfLet { .. }
        ));
        if name == "unit_pattern" {
            assert!(
                matches!(pattern.as_ref(),frontend::Pattern::Tuple(fields) if fields.is_empty())
            );
        } else if name == "return_before_semicolon" {
            assert!(matches!(pattern.as_ref(), frontend::Pattern::Bool(true)));
        } else {
            assert!(matches!(
                pattern.as_ref(),
                frontend::Pattern::Variant { .. }
            ));
        }
        if name == "return_before_semicolon" {
            assert!(body.statements.is_empty() && body.tail.is_none());
        } else {
            assert!(!body.statements.is_empty() || body.tail.is_some());
        }
    }
}
#[test]
fn real_iteration_plans_have_owned_downcasts_and_false_breaks_to_the_actual_loop() {
    let mut counts = (0, 0, 0);
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        let program = frontend::parse(case.source)
            .unwrap_or_else(|e| panic!("{} parse/resolve:{e}", case.name));
        if case.name.starts_with("macro_") {
            assert!(program.functions.iter().any(|f| f.lexical_scope.is_some()));
        }
        let signatures = frontend::signature_table(&program)
            .unwrap_or_else(|e| panic!("{} signatures:{e}", case.name));
        let mut loops = 0;
        let mut enums = 0;
        let mut sparse = Vec::new();
        for index in 0..program.functions.len() {
            let function = frontend::type_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} checker:{e}", case.name));
            function.validate().unwrap();
            for expression in &function.exprs {
                let ir::ExprKind::Loop { id, body } = &expression.kind else {
                    continue;
                };
                let [ir::Stmt::Expr(match_id)] = body.statements.as_slice() else {
                    continue;
                };
                let ir::ExprKind::Match { arms, .. } = &function.exprs[match_id.0].kind else {
                    continue;
                };
                assert_eq!(
                    arms.len(),
                    2,
                    "{} actual selected arm plus semantic false edge",
                    case.name
                );
                assert!(matches!(arms[1].pattern, ir::MatchPattern::Wildcard));
                assert!(
                    matches!(function.exprs[arms[1].body.0].kind,ir::ExprKind::Break { target,value:None } if target==*id),
                    "{} false edge must break this exact LoopId",
                    case.name
                );
                assert!(body.tail.is_none());
                if let ir::MatchPattern::EnumVariant {
                    ty,
                    variant,
                    bindings,
                } = &arms[0].pattern
                {
                    enums += 1;
                    assert!(*variant < ty.def().variants.len());
                    for binding in bindings {
                        assert!(!function.params.contains(&binding.local));
                        assert!(
                            matches!(binding.projection.first(),Some(ir::Projection::Downcast(owner)) if owner.owner==ty.def().id && owner.index==*variant)
                        );
                        assert!(
                            binding
                                .projection
                                .iter()
                                .skip(1)
                                .all(|p| matches!(p, ir::Projection::Field(_)))
                        );
                        if case.name == "sparse_suffix" {
                            sparse.push(binding.projection.clone());
                        }
                    }
                }
                if matches!(
                    case.name,
                    "dead_following_value" | "never_condition_unit_return"
                ) {
                    assert_eq!(
                        expression.completion,
                        ir::Completion::Never,
                        "{} actual loop cannot complete even though source Unit obligations remain",
                        case.name
                    );
                }
                loops += 1;
            }
        }
        assert_eq!(
            loops,
            if case.name == "nested_labels" { 2 } else { 1 },
            "{} exact while-let plans",
            case.name
        );
        if case.name == "sparse_suffix" {
            assert_eq!(sparse.len(), 2);
            assert!(matches!(
                sparse[0].as_slice(),
                [ir::Projection::Downcast(_), ir::Projection::Field(0)]
            ));
            assert!(matches!(
                sparse[1].as_slice(),
                [ir::Projection::Downcast(_), ir::Projection::Field(2)]
            ));
        }
        counts.0 += 1;
        counts.1 += loops;
        counts.2 += enums;
    }
    assert_eq!(counts, (18, 19, 12));
}
#[test]
fn ordinary_scope_loopback_body_and_source_unit_failures_never_become_unsupported() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Invalid)
    {
        let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(e) => e,
            Ok(_) => panic!("{} ordinary stock-invalid source accepted", case.name),
        };
        let diagnostic = failure.to_string();
        assert!(!diagnostic.is_empty());
        assert!(
            !diagnostic.contains("unsupported:"),
            "{} ordinary failure is not a capability boundary:{diagnostic}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 10);
}
#[test]
fn valid_closed_while_let_patterns_keep_precise_parser_and_checker_phases() {
    let mut counts = (0, 0);
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Unsupported)
    {
        assert!(case.stdout.is_some());
        let prepared = source::prepare_text(
            "while_let.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed = frontend::parse_prepared(&prepared);
        if matches!(
            case.name,
            "let_chain_boundary" | "reference_binding_boundary" | "or_boundary"
        ) {
            let error = match parsed {
                Err(e) => e,
                Ok(_) => panic!("{} parser boundary unexpectedly admitted", case.name),
            };
            assert_eq!(error.kind, source::SourceErrorKind::Unsupported);
            counts.0 += 1;
        } else {
            frontend::resolve(&parsed.unwrap()).unwrap();
            let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(e) => e,
                Ok(_) => panic!("{} checker boundary admitted", case.name),
            };
            let expected = match case.name {
                "refutable_payload_boundary" => "nested refutable literal patterns are unsupported",
                "noncopy_array_binding_boundary" => {
                    "non-Copy bindings through array patterns are unsupported"
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
    assert_eq!(counts, (3, 2));
}
#[test]
fn native_while_let_workers1_8_check_build_no_compiler_fallback() {
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
                        empty_main_control(&case),
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
    assert_eq!(gates, 132);
    assert_eq!(modes, (36, 36, 20, 20, 10, 10));
    eprintln!(
        "NATIVE WHILE-LET:132 cells, modes{modes:?};36 executed native main images (six deliberately empty main executions for the three exact completion controls, never execution of divergent value functions);20 stock-valid Unsupported cells are boundaries, not Rust-invalid parity or full Rust support"
    );
}
