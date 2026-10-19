//! Genuine let-else with persistent success bindings and a Never-typed failure expression.
//! A non-completing Unit-valued while is not a valid divergent else.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact baseline16 stock JSON SHA256:
// b0352c5b9d0e3f9e4a2a274e87042351cab320e56e412665a436062192297924
// Exact full33 stock JSON SHA256 before native expectations:
// e98609ae33117bf99686bf68478dc703b184678b9fb4c88e4cb843fe3d448dfd
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
            name: "success_escape_shadow",
            source: r#"enum Choice{Some(i32),None}fn main(){let x=40;let Choice::Some(x)=Choice::Some(x+2)else{return;};if x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "else_outer_binding",
            source: r#"enum Choice{Some(i32),None}fn main(){let x=42;let Choice::Some(x)=Choice::None else{if x==42{println!("ok");}return;};}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "conditional_noncopy_reuse",
            source: r#"struct Inner{x:i32}enum Choice{First(Inner),Second(Inner)}fn run(value:Choice)->i32{let Choice::First(inner)=value else{return match value{Choice::Second(inner)=>inner.x,_=>0};};inner.x}fn main(){if run(Choice::First(Inner{x:40}))+run(Choice::Second(Inner{x:2}))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "else_continue",
            source: r#"enum Choice{Some(i32),None}fn main(){let mut n=0;while n<2{n=n+1;let Choice::Some(x)=Choice::None else{continue;};}if n==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "never_else",
            source: r#"enum Choice{Some(i32),None}fn main(){let Choice::Some(x)=Choice::Some(42)else{loop{}};if x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "else_binding_not_available",
            source: r#"enum Choice{Some(i32),None}fn main(){let Choice::Some(x)=Choice::None else{let n=x;return;};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "nondivergent_else",
            source: r#"enum Choice{Some(i32),None}fn main(){let Choice::Some(x)=Choice::Some(42)else{0};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "partial_move_after_success",
            source: r#"struct Inner{x:i32}enum Choice{Some(Inner),None}fn run(value:Choice){let Choice::Some(inner)=value else{return;};let next=value;}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unit_else_infinite_while",
            source: r#"enum Choice{Some(i32),None}fn main(){let Choice::Some(x)=Choice::Some(42)else{while let _=1{}};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "never_initializer_missing_tail",
            source: r#"fn f()->i32{let true=(loop{})else{return 7;};}fn main(){}"#,
            stdout: Some(""),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "prior_return_never_initializer",
            source: r#"fn f()->i32{return 7;let true=(loop{})else{return 8;};}fn main(){}"#,
            stdout: Some(""),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unparenthesized_block_initializer",
            source: r#"fn main(){let _={1}else{return;};}"#,
            stdout: None,
            stock_diagnostic: Some("right curly brace"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "parenthesized_block_initializer",
            source: r#"fn main(){let _=({1})else{return;};println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unparenthesized_lazy_and",
            source: r#"fn main(){let true=true&&true else{return;};}"#,
            stdout: None,
            stock_diagnostic: Some("cannot be directly assigned"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unparenthesized_lazy_or",
            source: r#"fn main(){let true=true||false else{return;};}"#,
            stdout: None,
            stock_diagnostic: Some("cannot be directly assigned"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "parenthesized_lazy_and",
            source: r#"fn main(){let true=(true&&true)else{return;};println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only",
            source: r#"enum Choice{Some(i32),None}fn make()->Choice{println!("once");Choice::Some(42)}fn main(){let Choice::Some(x)=make()else{return;};if x==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_captured_binding",
            source: r#"enum Choice{Some(i32),None}fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{let Choice::Some($n)=$p else{return;};$n}};}let value=Choice::Some(42);let answer=pick!(value,n);if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_marked_shadow",
            source: r#"enum Choice{Some(i32),None}fn main(){macro_rules! pick{($p:ident,$n:ident)=>{{let Choice::Some(x)=$p else{return;};x+$n}};}let x=2;let value=Choice::Some(40);let answer=pick!(value,x);if answer==42{if x==2{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "imported_variant",
            source: r#"mod values{pub enum Choice{Some(i32),None}}use values::Choice::Some as Present;fn main(){let Present(x)=Present(42)else{return;};if x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "partial_move_omitted_sibling",
            source: r#"struct Inner{x:i32}struct Pair{a:Inner,b:Inner}fn main(){let value=Pair{a:Inner{x:40},b:Inner{x:2}};let Pair{a,..}=value else{return;};if a.x+value.b.x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "sparse_enum_suffix",
            source: r#"enum Choice{Some(i32,i32,i32),None}fn main(){let Choice::Some(a,..,z)=Choice::Some(40,7,2)else{return;};if a+z==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "else_break",
            source: r#"enum Choice{Some(i32),None}fn main(){while true{let Choice::Some(x)=Choice::None else{break;};}println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "escaping_noncopy_return",
            source: r#"struct Inner{x:i32}enum Choice{Some(Inner),None}fn take(value:Choice)->Inner{let Choice::Some(inner)=value else{return Inner{x:0};};inner}fn main(){let item=take(Choice::Some(Inner{x:42}));if item.x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "array_copy_selected",
            source: r#"fn main(){let [a,..,z]=[40,7,2]else{return;};if a+z==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "tuple_binding_escape",
            source: r#"fn main(){let (a,b)=(40,2)else{return;};if a+b==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "initializer_before_binding_invalid",
            source: r#"enum Choice{Some(i32),None}fn main(){let Choice::Some(x)=Choice::Some(x)else{return;};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "dead_else_checked_invalid",
            source: r#"fn main(){let _=1 else{missing();loop{}};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "duplicate_binding_invalid",
            source: r#"fn main(){let (a,a)=(40,2)else{return;};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0416]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "refutable_payload_boundary",
            source: r#"enum Choice{Some(i32),None}fn main(){let Choice::Some(42)=Choice::Some(42)else{return;};println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "reference_binding_boundary",
            source: r#"enum Choice{Some(i32),None}fn main(){let value=Choice::Some(42);let Choice::Some(ref x)=value else{return;};if *x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "or_boundary",
            source: r#"enum Choice{Some(i32),Other(i32)}fn main(){let (Choice::Some(x)|Choice::Other(x))=Choice::Some(42)else{return;};if x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "noncopy_array_binding_boundary",
            source: r#"struct Inner{x:i32}fn main(){let value=[Inner{x:42}];let [a]=value else{return;};if a.x==42{println!("ok");}}"#,
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
            "jr-let-else-{}-{nonce}-{}",
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
        "empty oracle only for exact measured empty-main controls"
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
        "never_initializer_missing_tail" | "prior_return_never_initializer"
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
        "LET-ELSE BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=let_else_oracle",
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
    assert_eq!(partition, (21, 10, 2));
}
#[test]
fn stock_let_else_mapping_ownership_and_valid_unsupported_oracle_edition2024() {
    stock_all();
}

#[test]
fn source_let_else_retains_real_boxed_pattern_initializer_and_failure_block() {
    for name in [
        "success_escape_shadow",
        "else_outer_binding",
        "never_initializer_missing_tail",
        "partial_move_omitted_sibling",
        "tuple_binding_escape",
    ] {
        let case = cases().into_iter().find(|c| c.name == name).unwrap();
        let prepared = source::prepare_text(
            "let_else.rs",
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
            .find(|s| matches!(s, frontend::Stmt::LetElse { .. }))
            .expect("genuine source LetElse, never a fabricated source Match or IfLet");
        let frontend::Stmt::LetElse {
            pattern,
            ty: _,
            value,
            else_block,
        } = statement
        else {
            unreachable!()
        };
        assert!(!matches!(
            value.as_ref(),
            frontend::Expr::Match { .. } | frontend::Expr::IfLet { .. }
        ));
        assert!(!else_block.statements.is_empty() || else_block.tail.is_some());
        match name {
            "never_initializer_missing_tail" => {
                assert!(matches!(pattern.as_ref(), frontend::Pattern::Bool(true)))
            }
            "partial_move_omitted_sibling" => assert!(matches!(
                pattern.as_ref(),
                frontend::Pattern::Struct { rest: true, .. }
            )),
            "tuple_binding_escape" => assert!(
                matches!(pattern.as_ref(),frontend::Pattern::Tuple(parts) if parts.len()==2)
            ),
            _ => assert!(matches!(
                pattern.as_ref(),
                frontend::Pattern::Variant { .. }
            )),
        }
    }
}
fn all_blocks(function: &ir::CheckedFunction) -> Vec<&ir::TypedBlock> {
    let mut blocks = vec![&function.body];
    for expression in &function.exprs {
        match &expression.kind {
            ir::ExprKind::Block(block)
            | ir::ExprKind::Loop { body: block, .. }
            | ir::ExprKind::LabeledBlock { body: block, .. } => blocks.push(block),
            ir::ExprKind::If {
                then_block,
                else_block,
                ..
            } => {
                blocks.push(then_block);
                blocks.push(else_block);
            }
            _ => {}
        }
    }
    let mut at = 0;
    while at < blocks.len() {
        let block = blocks[at];
        at += 1;
        for statement in &block.statements {
            if let ir::TypedStmt::While { body, .. } = statement {
                blocks.push(body);
            }
        }
    }
    blocks
}
#[test]
fn checked_let_else_has_persistent_success_locals_and_never_typed_failure() {
    let mut counts = (0, 0);
    for case in cases().into_iter().filter(|c| {
        c.outcome == Outcome::Runtime && !matches!(c.name, "refutable_payload_boundary")
    }) {
        let program = frontend::parse(case.source)
            .unwrap_or_else(|e| panic!("{} parse/resolve:{e}", case.name));
        if case.name.starts_with("macro_") {
            assert!(program.functions.iter().any(|f| f.lexical_scope.is_some()));
        }
        let signatures = frontend::signature_table(&program)
            .unwrap_or_else(|e| panic!("{} signatures:{e}", case.name));
        let mut plans = 0;
        let mut sparse = Vec::new();
        for index in 0..program.functions.len() {
            let function = frontend::type_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} checker:{e}", case.name));
            function.validate().unwrap();
            for block in all_blocks(&function) {
                for statement in &block.statements {
                    let ir::TypedStmt::LetElse {
                        value,
                        ty,
                        pattern,
                        else_branch,
                    } = statement
                    else {
                        continue;
                    };
                    assert_ne!(value, else_branch);
                    let failure = &function.exprs[else_branch.0];
                    assert_eq!(
                        failure.ty,
                        ir::Ty::Never,
                        "{} else must be Never-typed, not merely non-completing Unit",
                        case.name
                    );
                    assert_eq!(failure.completion, ir::Completion::Never);
                    assert!(
                        function.exprs[value.0].ty == *ty
                            || function.exprs[value.0].ty == ir::Ty::Never
                    );
                    let bindings = match pattern {
                        ir::MatchPattern::EnumVariant {
                            ty: enumeration,
                            variant,
                            bindings,
                        } => {
                            assert_eq!(ty, &ir::Ty::Enum(enumeration.clone()));
                            assert!(*variant < enumeration.def().variants.len());
                            for binding in bindings {
                                assert!(
                                    matches!(binding.projection.first(),Some(ir::Projection::Downcast(id)) if id.owner==enumeration.def().id && id.index==*variant)
                                );
                                assert!(
                                    binding
                                        .projection
                                        .iter()
                                        .skip(1)
                                        .all(|p| matches!(p, ir::Projection::Field(_)))
                                );
                            }
                            bindings.as_slice()
                        }
                        ir::MatchPattern::Aggregate {
                            ty: domain,
                            bindings,
                        } => {
                            assert_eq!(ty, domain);
                            bindings.as_slice()
                        }
                        ir::MatchPattern::Binding(local) => {
                            assert!(!function.params.contains(local));
                            &[]
                        }
                        _ => &[],
                    };
                    let mut locals = std::collections::HashSet::new();
                    for binding in bindings {
                        assert!(!function.params.contains(&binding.local));
                        assert!(locals.insert(binding.local));
                        if matches!(case.name, "sparse_enum_suffix" | "array_copy_selected") {
                            sparse.push(binding.projection.clone());
                        }
                    }
                    plans += 1;
                }
            }
        }
        assert_eq!(
            plans, 1,
            "{} one genuine persistent-binding statement",
            case.name
        );
        match case.name {
            "sparse_enum_suffix" => {
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
            "array_copy_selected" => assert_eq!(
                sparse,
                vec![
                    vec![ir::Projection::ConstantIndex(0)],
                    vec![ir::Projection::ConstantIndex(2)]
                ]
            ),
            _ => {}
        }
        counts.0 += 1;
        counts.1 += plans;
    }
    assert_eq!(counts, (20, 20));
}
#[test]
fn source_grammar_else_scope_divergence_and_moves_remain_ordinary_invalid() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Invalid)
    {
        let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(e) => e,
            Ok(_) => panic!("{} ordinary invalid source accepted", case.name),
        };
        let diagnostic = failure.to_string();
        assert!(!diagnostic.is_empty());
        assert!(
            !diagnostic.contains("unsupported:"),
            "{} ordinary error is not a capability boundary:{diagnostic}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 10);
}
#[test]
fn valid_closed_let_else_patterns_pin_actual_parser_and_checker_phases() {
    let mut counts = (0, 0);
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Unsupported)
    {
        assert!(case.stdout.is_some());
        let prepared = source::prepare_text(
            "let_else.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed = frontend::parse_prepared(&prepared);
        if matches!(case.name, "or_boundary") {
            let error = match parsed {
                Err(e) => e,
                Ok(_) => panic!("{} parser boundary admitted", case.name),
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
    assert_eq!(counts, (1, 1));
}
#[test]
fn native_let_else_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(modes, (42, 42, 20, 20, 4, 4));
    eprintln!(
        "NATIVE LET-ELSE:132 cells, modes{modes:?};40 main images (four deliberately empty executions for two exact Never-initializer controls, divergent functions unexecuted);12 stock-valid Unsupported cells are boundaries, not Rust-invalid parity or full Rust support"
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
