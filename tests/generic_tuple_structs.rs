//! Generic nonempty tuple structs with genuine source parameters and application-qualified nominal IR.
//! Stock lexical/semantic acceptance and explicit native boundaries are recorded separately.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Final32 stock JSON SHA256, measured before native expectations:
// 37c21394dc18eb320a15885a12001ec61a2c78c792e360826840a685d66f7885
// Original32 nominal-bound exploration retained separately as stock-only evidence:
// 4441f981f1365434038c373fe4974f38d6a5b56b5bd29baf837192773346c096
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
            name: "explicit",
            source: r#"struct Wrap<T>(T);fn main(){let x=Wrap::<i32>(42);if x.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "inferred",
            source: r#"struct Wrap<T>(T);fn main(){let value=Wrap(42);if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "expected",
            source: r#"struct Wrap<T>(T);fn main(){let x:Wrap<i32>=Wrap(42);if x.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_forward",
            source: r#"struct Wrap<T>(T);fn unwrap<T>(p:Wrap<T>)->T{let Wrap(x)=p;x}fn main(){if unwrap(Wrap(42))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "pair_generic_fn",
            source: r#"struct Pair<T,U>(T,U);fn first<T,U>(p:Pair<T,U>)->T{let Pair(a,_)=p;a}fn main(){if first(Pair(42,true))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "rest_match",
            source: r#"struct Pack<T>(T,i32,T);fn main(){let n=match Pack::<i32>(40,7,2){Pack(a,..,z)=>a+z};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "rest_if_let",
            source: r#"struct Pack<T>(T,i32,T);fn main(){if let Pack(a,..,z)=Pack(40,7,2){if a+z==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "rest_while_let",
            source: r#"struct Pack<T>(T,i32,T);fn main(){while let Pack(a,..,z)=Pack(40,7,2){if a+z==42{println!("ok");}break;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "rest_let_else",
            source: r#"struct Pack<T>(T,i32,T);fn main(){let Pack(a,..,z)=Pack(40,7,2)else{return;};if a+z==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro",
            source: r#"struct Wrap<T>(T);fn main(){macro_rules! make{($x:ident)=>{Wrap($x)};}let n=42;let value=make!(n);if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "import_alias",
            source: r#"mod values{pub struct Wrap<T>(pub T);}use values::Wrap as Boxed;fn main(){let Boxed(n)=Boxed(42);if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_omitted",
            source: r#"struct Item{x:i32}struct Pair<T,U>(T,U);fn main(){let value=Pair(Item{x:40},Item{x:2});let Pair(a,..)=value;if a.x+value.1.x==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "field_mutation",
            source: r#"struct Wrap<T>(T);fn main(){let mut value=Wrap(40);value.0=value.0+2;if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "eval_once",
            source: r#"struct Pair<T,U>(T,U);fn left()->i32{println!("left");40}fn right()->i32{println!("right");2}fn main(){let value=Pair(left(),right());if value.0+value.1==42{println!("ok");}}"#,
            stdout: Some("left\nright\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_nominal",
            source: r#"struct Wrap<T>(T);struct Pair<T,U>(T,U);fn main(){let Wrap(Pair(a,b))=Wrap(Pair(40,2));if a+b==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "copy_bound",
            source: r#"struct Wrap<T>(T);fn take<T:Copy>(value:Wrap<T>)->T{value.0}fn main(){if take(Wrap(42))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "explicit_pattern",
            source: r#"struct Pair<T,U>(T,U);fn main(){let Pair::<i32,bool>(a,b)=Pair(42,true);if b{if a==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "typealias_type_use",
            source: r#"struct Wrap<T>(T);type Number=Wrap<i32>;fn main(){let value:Number=Wrap(42);if value.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "application_mismatch",
            source: r#"struct Wrap<T>(T);fn take(p:Wrap<bool>)->bool{p.0}fn main(){let n=take(Wrap::<i32>(42));}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "arity",
            source: r#"struct Pair<T,U>(T,U);fn main(){let x=Pair::<i32,bool>(42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0061]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "bound",
            source: r#"struct Wrap<T>(T);fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn main(){let x=dup(Wrap(42));}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0277]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unused_generic_body",
            source: r#"struct Wrap<T>(T);fn bad<T>(p:Wrap<T>)->i32{p.0}fn main(){println!("ok");}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "typealias_call",
            source: r#"struct Wrap<T>(T);type Number=Wrap<i32>;fn main(){let x=Number(42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0423]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "private_constructor",
            source: r#"mod values{pub struct Wrap<T>(T);}fn main(){let x=values::Wrap(42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0603]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "private_field",
            source: r#"mod values{pub struct Wrap<T>(T);pub fn make()->Wrap<i32>{Wrap(42)}}fn main(){let x=values::make();let y=x.0;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0616]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "noncopy_repeat_move",
            source: r#"struct Item{x:i32}struct Wrap<T>(T);fn main(){let a=Wrap(Item{x:42});let b=a;let c=a;}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "rigid_generic_wrong",
            source: r#"struct Pair<T,U>(T,U);fn bad<T,U>(p:Pair<T,U>)->T{p.1}fn main(){println!("ok");}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_nominal_pattern",
            source: r#"struct Wrap<T>(T);struct Other<T>(T);fn main(){let Other(x)=Wrap(42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "zero_tuple_boundary",
            source: r#"struct Empty();fn main(){let x=Empty();println!("ok");}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "first_class_boundary",
            source: r#"struct Wrap<T>(T);fn main(){let make=Wrap::<i32>;let x=make(42);if x.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "const_generic_boundary",
            source: r#"struct Wrap<const N:usize>(i32);fn main(){let x=Wrap::<3>(42);if x.0==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "reference_pattern_boundary",
            source: r#"struct Wrap<T>(T);fn main(){let value=Wrap(42);let Wrap(ref x)=value;if *x==42{println!("ok");}}"#,
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
            "jr-generic-tuples-{}-{nonce}-{}",
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
        "GENERIC TUPLE BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=generic_tuple_oracle",
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
    assert_eq!(counts, (22, 10));
    assert_eq!(partition, (19, 10, 3));
}
#[test]
fn stock_generic_tuple_oracle_edition2024_actual_toolchain() {
    stock_all();
}
#[test]
fn source_generic_tuple_declarations_keep_real_parameters_and_numeric_fields() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        let prepared = source::prepare_text(
            "generic_tuples.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed =
            frontend::parse_prepared(&prepared).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let tuples: Vec<_> = parsed
            .program
            .structs
            .iter()
            .filter(|d| d.shape == frontend::StructShape::Tuple)
            .collect();
        assert!(!tuples.is_empty(), "{}", case.name);
        for declaration in tuples {
            assert!(!declaration.type_params.is_empty());
            assert!(!declaration.fields.is_empty());
            declaration
                .name
                .check(parsed.program.names.as_ref())
                .unwrap();
            for parameter in &declaration.type_params {
                parsed.program.names.spelling(*parameter).unwrap();
            }
            for (index, field) in declaration.fields.iter().enumerate() {
                assert!(matches!(field.name,frontend::FieldName::Tuple(actual) if actual==index));
            }
        }
        count += 1;
    }
    assert_eq!(count, 19);
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
fn inspect_pattern(pattern: &ir::MatchPattern, sparse: &mut Vec<Vec<ir::Projection>>) {
    match pattern {
        ir::MatchPattern::Aggregate { ty, bindings } => {
            assert!(matches!(ty, ir::Ty::Struct(_)));
            for binding in bindings {
                assert!(
                    binding
                        .projection
                        .iter()
                        .all(|p| matches!(p, ir::Projection::Field(_)))
                );
                sparse.push(binding.projection.clone());
            }
        }
        ir::MatchPattern::Wildcard | ir::MatchPattern::Binding(_) => {}
        _ => panic!("generic tuple plans must not masquerade as enum downcasts"),
    }
}
#[test]
fn generic_tuple_templates_and_concrete_domains_preserve_application_and_field_plans() {
    let mut counts = (0, 0, 0);
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        let program = frontend::parse(case.source).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let signatures =
            frontend::signature_table(&program).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let mut sparse = Vec::new();
        let mut constructors = 0;
        for index in 0..program.functions.len() {
            // This public checker includes universal template ownership, not just a concrete i32 instance.
            frontend::check_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} function{index}:{e}", case.name));
            let function =
                frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
            if program.functions[index].type_params.is_empty() {
                function.validate().unwrap();
            } else {
                counts.1 += 1;
                assert!(function.params.iter().any(|p|matches!(&function.locals[p.0].ty,ir::Ty::Struct(s) if s.def().type_args.iter().any(|t|matches!(t,ir::Ty::Param(_))))));
                assert!(matches!(function.result, ir::Ty::Param(_)));
            }
            for expression in &function.exprs {
                match &expression.kind {
                    ir::ExprKind::Struct { ty, fields } if !ty.def().type_args.is_empty() => {
                        assert!(!ty.def().fields.is_empty());
                        assert_eq!(fields.len(), ty.def().fields.len());
                        assert!(
                            fields
                                .iter()
                                .enumerate()
                                .all(|(i, (actual, _))| i == *actual)
                        );
                        assert_eq!(expression.ty, ir::Ty::Struct(ty.clone()));
                        constructors += 1;
                    }
                    ir::ExprKind::Match { arms, .. } => {
                        for arm in arms {
                            inspect_pattern(&arm.pattern, &mut sparse);
                        }
                    }
                    _ => {}
                }
            }
            for block in all_blocks(&function) {
                for statement in &block.statements {
                    match statement {
                        ir::TypedStmt::LetPattern {
                            ty: ir::Ty::Struct(_),
                            bindings,
                            ..
                        } => {
                            for binding in bindings {
                                assert!(
                                    binding
                                        .projection
                                        .iter()
                                        .all(|p| matches!(p, ir::Projection::Field(_)))
                                );
                                if case.name == "noncopy_omitted" {
                                    sparse.push(binding.projection.clone());
                                }
                            }
                        }
                        ir::TypedStmt::LetElse { ty, pattern, .. } => {
                            assert!(matches!(ty, ir::Ty::Struct(_)));
                            inspect_pattern(pattern, &mut sparse);
                        }
                        _ => {}
                    }
                }
            }
        }
        assert!(
            constructors > 0,
            "{} must have an actual checked generic nominal constructor",
            case.name
        );
        if case.name.starts_with("rest_") {
            assert_eq!(
                sparse,
                vec![
                    vec![ir::Projection::Field(0)],
                    vec![ir::Projection::Field(2)]
                ],
                "{}",
                case.name
            );
        }
        if case.name == "noncopy_omitted" {
            assert_eq!(sparse, vec![vec![ir::Projection::Field(0)]]);
        }
        counts.0 += 1;
        counts.2 += constructors;
    }
    assert_eq!(counts.0, 19);
    assert_eq!(counts.1, 3);
    assert!(counts.2 >= 18);
}
#[test]
fn generic_tuple_wrong_applications_bounds_privacy_and_unused_bodies_are_invalid() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Invalid)
    {
        let failure = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(e) => e,
            Ok(_) => panic!("{} accepted", case.name),
        };
        let message = failure.to_string();
        assert!(!message.is_empty());
        assert!(
            !message.contains("unsupported:"),
            "{} ordinary invalid is not a capability boundary:{message}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 10);
}
#[test]
fn valid_closed_tuple_forms_keep_precise_parser_and_resolution_boundaries() {
    let mut counts = (0, 0);
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Unsupported)
    {
        let prepared = source::prepare_text(
            "generic_tuples.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed = frontend::parse_prepared(&prepared);
        if case.name == "first_class_boundary" {
            let error = match frontend::resolve(&parsed.unwrap()) {
                Err(e) => e,
                Ok(_) => panic!("first-class constructor accepted"),
            };
            assert!(
                error
                    .contains("unsupported: first-class tuple struct constructors are unsupported"),
                "{error}"
            );
            counts.1 += 1;
        } else {
            let error = match parsed {
                Err(e) => e,
                Ok(_) => panic!("{} parser boundary accepted", case.name),
            };
            assert_eq!(
                error.kind,
                source::SourceErrorKind::Unsupported,
                "{}:{error}",
                case.name
            );
            counts.0 += 1;
        }
    }
    assert_eq!(counts, (2, 1));
}
#[test]
fn native_generic_tuple_structs_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(gates, 128);
    assert_eq!(modes, (38, 38, 20, 20, 6, 6));
    eprintln!(
        "NATIVE GENERIC TUPLES:128 cells, modes{modes:?};36 nonempty-output main images;16 stock-valid Unsupported cells are boundaries, not Rust-invalid parity or full Rust support"
    );
}
