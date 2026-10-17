//! Ordered nested pattern predicates with exact domains and success-only bindings.
//! Stock-invalid/native-Unsupported coverage boundaries are distinct from ordinary-invalid parity.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact30 stock JSON SHA256, measured before native expectations:
// 04860b9744e16be425b8c3ed626e90098cfbeb4a07aa765dcf0e10931b8070b2
// Original11 design sources retained as the first11 cases in unchanged order.
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
            name: "ordered_same_variant",
            source: r#"struct Item{n:i32}enum E{Some(bool,Item),None}fn run(v:E)->i32{match v{E::Some(true,x)=>x.n,E::Some(_,x)=>x.n+2,E::None=>0}}fn main(){if run(E::Some(false,Item{n:40}))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_tag",
            source: r#"enum Inner{A(i32),B}enum Outer{Wrap(Inner),None}fn run(v:Outer)->i32{match v{Outer::Wrap(Inner::A(42))=>40,Outer::Wrap(_)=>2,Outer::None=>0}}fn main(){if run(Outer::Wrap(Inner::A(42)))+run(Outer::Wrap(Inner::B))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "record_tuple_bool_int",
            source: r#"struct R{pair:(bool,i32),n:i32}fn main(){let v=R{pair:(true,7),n:42};let n=match v{R{pair:(true,7),n}=>n,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "if_let_nested",
            source: r#"enum E{A(bool,i32),B}fn main(){if let E::A(true,n)=E::A(true,42){if n==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "while_let_nested",
            source: r#"enum E{A(bool,i32),B}fn main(){let mut state=E::A(true,42);while let E::A(true,n)=state{if n==42{println!("ok");}state=E::B;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "let_else_failure_reuse",
            source: r#"struct Item{n:i32}enum E{A(bool,Item),B}fn run(v:E)->i32{let E::A(true,item)=v else{return match v{E::A(_,item)=>item.n,E::B=>0};};item.n}fn main(){if run(E::A(false,Item{n:42}))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_predicate_borrow_only",
            source: r#"struct Item{n:i32}struct R{item:Item,flag:bool}fn main(){let r=R{item:Item{n:42},flag:false};if let R{flag:true,..}=r{}let next=r;if next.item.n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "conditional_move_invalid",
            source: r#"struct Item{n:i32}enum E{A(bool,Item),B}fn run(v:E){if let E::A(true,item)=v{let n=item.n;}let later=v;}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "plain_let_refutable_invalid",
            source: r#"enum E{A(bool,i32),B}fn main(){let E::A(true,n)=E::A(true,42);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0005]"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "incomplete_nested_invalid",
            source: r#"enum E{A(bool,i32),B}fn run(v:E)->i32{match v{E::A(true,n)=>n,E::B=>0}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0004]"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "complete_bool_product",
            source: r#"enum E{A(bool,i32),B}fn run(v:E)->i32{match v{E::A(true,n)=>n,E::A(false,n)=>n,E::B=>0}}fn main(){if run(E::A(false,42))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "late_binding_failed_test",
            source: r#"struct Item{n:i32}enum E{A(Item,bool),B}fn run(v:E)->i32{match v{E::A(item,true)=>item.n,E::A(item,_)=>item.n+2,E::B=>0}}fn main(){if run(E::A(Item{n:40},false))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "signed_integer",
            source: r#"enum E{A(i32),B}fn main(){let n=match E::A(-7){E::A(-7)=>42,E::A(_)=>0,E::B=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wide_integer",
            source: r#"enum E{A(i128),B}fn main(){let n=match E::A(-170141183460469231731687303715884105728i128){E::A(-170141183460469231731687303715884105728i128)=>42,E::A(_)=>0,E::B=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_enum",
            source: r#"enum E<T>{A(T,bool),B}fn main(){let n=match E::<i32>::A(42,true){E::A(n,true)=>n,E::A(_,_)=>0,E::B=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_tuple",
            source: r#"struct Pair<T,U>(T,U);fn main(){let n=match Pair(true,42){Pair(true,n)=>n,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "array_scalar",
            source: r#"fn main(){let n=match [true,false,true]{[true,_,true]=>42,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "nested_tag_twice",
            source: r#"enum I{A(i32),B}enum E{A(I,I),B}fn main(){let n=match E::A(I::B,I::A(42)){E::A(I::A(_),I::A(n))=>n,E::A(I::B,I::A(n))=>n,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only",
            source: r#"enum E{A(bool,i32),B}fn make()->E{println!("once");E::A(true,42)}fn main(){let n=match make(){E::A(true,n)=>n,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro",
            source: r#"enum E{A(bool,i32),B}fn main(){macro_rules! pick{($v:ident,$n:ident)=>{{if let E::A(true,$n)=$v{$n}else{0}}};}let value=E::A(true,42);let answer=pick!(value,n);if answer==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "private_omitted",
            source: r#"mod values{pub struct R{pub flag:bool,secret:i32}pub fn make()->R{R{flag:true,secret:7}}}fn main(){if let values::R{flag:true,..}=values::make(){println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_array_copyfield",
            source: r#"struct Item{flag:bool,n:i32}fn main(){let a=[Item{flag:true,n:42}];if let [Item{flag:true,..}]=a{if a[0].n==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wrong_literal_type",
            source: r#"enum E{A(i32),B}fn main(){let n=match E::A(42){E::A(true)=>42,_=>0};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_nominal",
            source: r#"enum E{A(i32),B}enum Other{A(i32),B}fn main(){let n=match E::A(42){Other::A(42)=>42,_=>0};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "selected_private",
            source: r#"mod values{pub struct R{secret:bool}pub fn make()->R{R{secret:true}}}fn main(){if let values::R{secret:true}=values::make(){println!("ok");}}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0451]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "dead_arm_invalid",
            source: r#"enum E{A(bool),B}fn main(){let n=match E::A(true){_=>42,E::A(true)=>missing()};}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0425]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "guard_boundary",
            source: r#"enum E{A(bool,i32),B}fn main(){let n=match E::A(true,42){E::A(true,n) if n==42=>n,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "or_boundary",
            source: r#"enum E{A(bool,i32),B}fn main(){let n=match E::A(true,42){E::A(true,n)|E::A(false,n)=>n,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "reference_binding_boundary",
            source: r#"enum E{A(bool,i32),B}fn main(){let value=E::A(true,42);let n=match value{E::A(true,ref n)=>*n,_=>0};if n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "array_noncopy_selection_boundary",
            source: r#"struct Item{flag:bool,n:i32}fn main(){let value=[Item{flag:true,n:42}];let n=match value{[item]=>item.n};if n==42{println!("ok");}}"#,
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
            "jr-pattern-decisions-{}-{nonce}-{}",
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
        "PATTERN DECISIONS BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=pattern_decisions_oracle",
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
    assert_eq!(counts, (23, 7));
    assert_eq!(partition, (18, 5, 5, 2));
}
#[test]
fn stock_nested_pattern_decisions_oracle_edition2024_actual_toolchain() {
    stock_all();
}
fn all_blocks(function: &ir::CheckedFunction) -> Vec<&ir::Block> {
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
            if let ir::Stmt::While { body, .. } = statement {
                blocks.push(body);
            }
        }
    }
    blocks
}
// Independent projection walk pins exact tag domination, not merely a matching enum name.
fn projected_type(domain: &ir::Ty, path: &[ir::Projection], prior: &[ir::PatternTest]) -> ir::Ty {
    let mut current = domain.clone();
    let mut selected = None;
    for (index, projection) in path.iter().enumerate() {
        match projection {
            ir::Projection::Downcast(variant) => {
                let ir::Ty::Enum(enumeration) = &current else {
                    panic!("downcast requires an actual enum domain")
                };
                assert!(selected.is_none());
                assert_eq!(variant.owner, enumeration.def().id);
                assert!(variant.index < enumeration.def().variants.len());
                assert!(prior.iter().any(|test|test.projection==path[..index]&&matches!(&test.kind,ir::PatternTestKind::EnumVariant{ty,variant:index} if ty==enumeration&&*index==variant.index)),"downcast must have an earlier successful exact-prefix tag test");
                selected = Some(variant.index);
            }
            ir::Projection::Field(field) => {
                current = match &current {
                    ir::Ty::Enum(enumeration) => {
                        let variant = selected
                            .take()
                            .expect("enum field without a preceding downcast");
                        enumeration.def().variants[variant].fields[*field]
                            .ty
                            .clone()
                    }
                    ir::Ty::Struct(structure) => {
                        assert!(selected.is_none());
                        structure.def().fields[*field].ty.clone()
                    }
                    ir::Ty::Tuple(fields) => {
                        assert!(selected.is_none());
                        fields[*field].clone()
                    }
                    _ => panic!("field on a non-aggregate domain"),
                };
            }
            ir::Projection::ConstantIndex(index) => {
                assert!(selected.is_none());
                let ir::Ty::Array { element, len } = &current else {
                    panic!("constant index requires array domain")
                };
                assert!(*index < *len);
                current = element.as_ref().clone();
            }
            _ => panic!("decision paths must not acquire dereference or runtime-index authority"),
        }
    }
    current
}
fn check_decision(
    function: &ir::CheckedFunction,
    pattern: &ir::MatchPattern,
    case: &Case,
) -> usize {
    let ir::MatchPattern::Decision {
        ty,
        tests,
        bindings,
    } = pattern
    else {
        return 0;
    };
    assert!(!tests.is_empty());
    for (index, test) in tests.iter().enumerate() {
        assert!(
            !tests[..index]
                .iter()
                .any(|earlier| earlier.projection == test.projection),
            "duplicate predicate path"
        );
        let projected = projected_type(ty, &test.projection, &tests[..index]);
        match &test.kind {
            ir::PatternTestKind::Bool(_) => assert_eq!(projected, ir::Ty::Bool),
            ir::PatternTestKind::Int(value) => {
                value.validate().unwrap();
                assert_eq!(projected, ir::Ty::Int(value.ty));
            }
            ir::PatternTestKind::EnumVariant {
                ty: enumeration,
                variant,
            } => {
                assert_eq!(projected, ir::Ty::Enum(enumeration.clone()));
                assert!(*variant < enumeration.def().variants.len());
            }
        }
    }
    let mut locals = std::collections::HashSet::new();
    for binding in bindings {
        assert!(locals.insert(binding.local));
        assert!(!function.params.contains(&binding.local));
        assert_eq!(
            projected_type(ty, &binding.projection, tests),
            function.locals[binding.local.0].ty
        );
    }
    if case.name == "noncopy_array_copyfield" || case.name == "noncopy_predicate_borrow_only" {
        assert!(
            bindings.is_empty(),
            "predicate-only inspection cannot fabricate moves"
        );
    }
    if case.name == "late_binding_failed_test" {
        assert!(
            tests
                .iter()
                .any(|test| matches!(test.kind, ir::PatternTestKind::Bool(true))
                    && matches!(test.projection.last(), Some(ir::Projection::Field(1))))
        );
        assert!(
            bindings
                .iter()
                .any(|binding| matches!(binding.projection.last(), Some(ir::Projection::Field(0))))
        );
    }
    if case.name == "wide_integer" {
        assert!(tests.iter().any(|test|matches!(test.kind,ir::PatternTestKind::Int(ir::IntValue{ty:ir::IntTy::I128,bits}) if bits==1u128<<127)));
    }
    1
}
#[test]
fn checked_decision_domains_scalar_atoms_tag_dominance_and_real_success_bindings() {
    let mut count = 0;
    let mut decisions = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        let prepared = source::prepare_text(
            "decisions.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed =
            frontend::parse_prepared(&prepared).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let program = frontend::resolve(&parsed).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        if case.name == "macro" {
            assert!(program.functions.iter().any(|f| f.lexical_scope.is_some()));
        }
        let signatures = frontend::signature_table(&program).unwrap();
        let mut own_decisions = 0;
        for index in 0..program.functions.len() {
            frontend::check_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} function{index}:{e}", case.name));
            let function =
                frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
            function.validate().unwrap();
            for expression in &function.exprs {
                if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                    for arm in arms {
                        own_decisions += check_decision(&function, &arm.pattern, &case);
                    }
                }
            }
            for block in all_blocks(&function) {
                for statement in &block.statements {
                    if let ir::Stmt::LetElse {
                        ty,
                        pattern,
                        else_branch,
                        ..
                    } = statement
                    {
                        let ir::MatchPattern::Decision { ty: domain, .. } = pattern else {
                            panic!("nested refutable let-else requires a genuine decision")
                        };
                        assert_eq!(ty, domain);
                        assert_eq!(function.exprs[else_branch.0].ty, ir::Ty::Never);
                        assert_eq!(
                            function.exprs[else_branch.0].completion,
                            ir::Completion::Never
                        );
                        own_decisions += check_decision(&function, pattern, &case);
                    }
                }
            }
        }
        assert!(
            own_decisions > 0,
            "{} did not produce actual checked predicate IR",
            case.name
        );
        decisions += own_decisions;
        count += 1;
    }
    assert_eq!(count, 18);
    assert!(decisions >= 18);
}
#[test]
fn nested_decisions_keep_actual_type_privacy_move_and_dead_arm_errors_invalid() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Invalid)
    {
        let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(error) => error,
            Ok(_) => panic!("{} invalid accepted", case.name),
        };
        let message = error.to_string();
        assert!(!message.is_empty());
        assert!(
            !message.contains("unsupported:"),
            "{} ordinary error is not a capability boundary:{message}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 5);
}
#[test]
fn valid_and_stock_invalid_unsupported_boundaries_are_distinct_and_phase_pinned() {
    let mut counts = (0, 0, 0);
    for case in cases().into_iter().filter(|c| {
        matches!(
            c.outcome,
            Outcome::Unsupported | Outcome::InvalidUnsupported
        )
    }) {
        let prepared = source::prepare_text(
            "decisions.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed = frontend::parse_prepared(&prepared);
        if matches!(
            case.name,
            "guard_boundary" | "or_boundary" | "reference_binding_boundary"
        ) {
            let error = match parsed {
                Err(e) => e,
                Ok(_) => panic!("{} parser boundary admitted", case.name),
            };
            assert_eq!(error.kind, source::SourceErrorKind::Unsupported);
            counts.0 += 1;
        } else {
            frontend::resolve(&parsed.unwrap()).unwrap();
            let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
                Err(e) => e,
                Ok(_) => panic!("{} checker boundary admitted", case.name),
            };
            let message = error.to_string();
            assert!(message.contains("unsupported:"), "{}:{message}", case.name);
            let expected = match case.name {
                "complete_bool_product" | "incomplete_nested_invalid" => {
                    "nested pattern exhaustiveness requires an irrefutable variant or catch-all fallback"
                }
                "plain_let_refutable_invalid" => {
                    "refutable enum patterns in irrefutable positions are unsupported"
                }
                "array_noncopy_selection_boundary" => {
                    "non-Copy bindings through array patterns are unsupported"
                }
                _ => unreachable!(),
            };
            assert!(message.contains(expected), "{}:{message}", case.name);
            if case.outcome == Outcome::InvalidUnsupported {
                assert!(case.stdout.is_none());
                assert!(case.stock_diagnostic.is_some());
                counts.2 += 1;
            } else {
                assert!(case.stdout.is_some());
                counts.1 += 1;
            }
        }
    }
    assert_eq!(counts, (3, 2, 2));
}
#[test]
fn native_pattern_decisions_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(gates, 120);
    assert_eq!(modes, (36, 36, 10, 10, 10, 10, 4, 4));
    eprintln!(
        "NATIVE PATTERN DECISIONS:120 cells, modes{modes:?};36 nonempty-output main images;20 stock-valid Unsupported cells and8 stock-invalid/native-Unsupported cells are distinct boundaries, not ordinary-invalid parity or full Rust support"
    );
}
