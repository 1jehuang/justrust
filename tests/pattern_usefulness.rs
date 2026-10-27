//! Exact bounded constructor-specialization coverage over existing checked Decision patterns.
//! Stock-invalid lint/plain-let boundaries remain separate from ordinary non-exhaustiveness.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Exact32 stock JSON SHA256: 4783511d39d27e020eac9e6d4fc815e70f5a897ebd0576a5e84225b93d730817
// First10 exact source bytes/order retained from stock design oracle.
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
            name: "bool_product",
            source: r#"fn f(p:(bool,bool))->i32{match p{(true,_)=>1,(false,true)=>2,(false,false)=>3}}fn main(){if f((false,false))==3{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "enum_bool_product",
            source: r#"enum E{A(bool),B}fn f(p:E)->i32{match p{E::A(true)=>1,E::A(false)=>2,E::B=>3}}fn main(){if f(E::A(false))==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "missing_bool_product",
            source: r#"fn f(p:(bool,bool))->i32{match p{(true,_)=>1,(false,true)=>2}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("E0004"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "nested_enum_product",
            source: r#"enum I{A,B}enum E{One(I),Two}fn f(p:E)->i32{match p{E::One(I::A)=>1,E::One(I::B)=>2,E::Two=>3}}fn main(){if f(E::One(I::B))==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "subsumed_default",
            source: r#"fn f(p:(bool,bool))->i32{match p{(true,_)=>1,(true,false)=>2,(false,_)=>3}}fn main(){if f((true,false))==1{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "subsumed_deny",
            source: r#"#![deny(unreachable_patterns)]
fn f(p:(bool,bool))->i32{match p{(true,_)=>1,(true,false)=>2,(false,_)=>3}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("unreachable pattern"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "unreachable_body_still_checked",
            source: r#"fn f(p:(bool,bool))->i32{match p{_=>1,(true,false)=>missing}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("E0425"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "integer_other_required",
            source: r#"fn f(p:(i128,bool))->i32{match p{(0,_)=>1,(1,_)=>2}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("E0004"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "integer_default_product",
            source: r#"fn f(p:(i128,bool))->i32{match p{(0,true)=>1,(_,false)=>2,(_,true)=>3}}fn main(){if f((99,true))==3{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unit_struct_product",
            source: r#"struct U;fn f(p:(U,bool))->i32{match p{(U,true)=>1,(U,false)=>2}}fn main(){if f((U,false))==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "record_product",
            source: r#"struct R{a:bool,b:bool}fn f(r:R)->i32{match r{R{a:true,..}=>1,R{a:false,b:true}=>2,R{a:false,b:false}=>3}}fn main(){if f(R{a:false,b:false})==3{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "array_product",
            source: r#"fn f(a:[bool;2])->i32{match a{[true,_]=>1,[false,true]=>2,[false,false]=>3}}fn main(){if f([false,false])==3{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "collective_subsumption",
            source: r#"fn f(p:(bool,bool))->i32{match p{(true,_)=>1,(false,true)=>2,(_,true)=>99,(false,false)=>3}}fn main(){if f((false,true))==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "u8_other",
            source: r#"fn f(p:(u8,bool))->i32{match p{(0,_)=>1,(255,true)=>2,(_,false)=>3,(_,true)=>4}}fn main(){if f((7,true))==4{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "i128_extrema",
            source: r#"fn f(p:(i128,bool))->i32{match p{(-170141183460469231731687303715884105728,true)=>1,(170141183460469231731687303715884105727,_)=>2,(_,false)=>3,(_,true)=>4}}fn main(){if f((-170141183460469231731687303715884105728,true))==1{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "u128_extrema",
            source: r#"fn f(p:(u128,bool))->i32{match p{(340282366920938463463374607431768211455,true)=>1,(0,_)=>2,(_,false)=>3,(_,true)=>4}}fn main(){if f((340282366920938463463374607431768211455,true))==1{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "once_only",
            source: r#"fn source()->(bool,bool){println!("once");(false,true)}fn main(){let n=match source(){(true,_)=>1,(false,true)=>42,(false,false)=>3};if n==42{println!("ok");}}"#,
            stdout: Some("once\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "failed_predicate_noncopy",
            source: r#"struct Cell{n:i32}enum E{A(Cell,bool),B}fn f(e:E)->i32{match e{E::A(cell,true)=>cell.n,E::A(cell,false)=>cell.n+2,E::B=>0}}fn main(){if f(E::A(Cell{n:40},false))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_rigid",
            source: r#"struct Wrap<T>(T,bool);fn take<T>(w:Wrap<T>)->T{match w{Wrap(x,true)=>x,Wrap(x,false)=>x}}fn main(){if take(Wrap(42,true))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "if_let_client",
            source: r#"enum E{A(bool,i32),B}fn main(){let e=E::A(false,42);if let E::A(true,n)=e{println!("bad");}else{let n=match e{E::A(true,n)=>n,E::A(false,n)=>n,E::B=>0};if n==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "while_let_client",
            source: r#"enum E{A(bool,i32),B}fn main(){let mut e=E::A(true,42);while let E::A(true,n)=e{let n=match (false,n){(true,x)=>x+1,(false,x)=>x};if n==42{println!("ok");}e=E::B;}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "let_else_client",
            source: r#"enum E{A(bool,i32),B}fn f(e:E)->i32{let E::A(true,n)=e else{return match e{E::A(true,n)=>n+1,E::A(false,n)=>n,E::B=>0};};n}fn main(){if f(E::A(false,42))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_product",
            source: r#"fn main(){macro_rules! pick{($p:ident)=>{{match $p{(true,_)=>1,(false,true)=>2,(false,false)=>42}}};}let p=(false,false);if pick!(p)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "import_alias",
            source: r#"mod m{pub enum E{A(bool),B}}use m::E as Alias;fn f(e:Alias)->i32{match e{Alias::A(true)=>1,Alias::A(false)=>42,Alias::B=>0}}fn main(){if f(Alias::A(false))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "record_hole",
            source: r#"struct R{a:bool,b:bool}fn f(r:R)->i32{match r{R{a:true,..}=>1,R{a:false,b:true}=>2}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("E0004"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "array_hole",
            source: r#"fn f(a:[bool;2])->i32{match a{[true,_]=>1,[false,true]=>2}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("E0004"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "dead_body_type",
            source: r#"fn f(p:(bool,bool))->i32{match p{_=>1,(true,false)=>true}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("E0308"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "guard_boundary",
            source: r#"fn f(p:(bool,bool))->i32{match p{(a,b) if a=>1,_=>2}}fn main(){if f((false,true))==2{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "reference_boundary",
            source: r#"fn f(p:(bool,bool))->i32{match p{(ref a,_)=>1}}fn main(){if f((false,true))==1{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "plainlet_boundary",
            source: r#"enum E{A(bool),B}fn f(e:E){let E::A(true)=e;}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("E0005"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "or_boundary",
            source: r#"fn f(p:(bool,bool))->i32{match p{(true,_)|(false,_)=>42}}fn main(){if f((false,true))==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "enum_multiple_payload_variants",
            source: r#"enum E{A(bool),B(bool)}fn f(e:E)->i32{match e{E::A(true)=>1,E::A(false)=>2,E::B(true)=>3,E::B(false)=>4}}fn main(){if f(E::A(false))+f(E::B(true))==5{println!("ok");}}"#,
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
            "jr-pattern-usefulness-{}-{nonce}-{}",
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
        "PATTERN USEFULNESS BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=pattern_usefulness_oracle",
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
            if matches!(case.name, "subsumed_default" | "collective_subsumption") {
                assert!(
                    String::from_utf8_lossy(&result.stderr).contains("unreachable pattern"),
                    "{}: stock must retain warning category",
                    case.name
                );
            }
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
    assert_eq!(counts, (24, 8));
    assert_eq!(partition, (23, 6, 1, 2));
}
#[test]
fn stock_nested_pattern_usefulness_oracle_edition2024_actual_toolchain() {
    stock_all();
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
                        enumeration.def().variants[variant]
                            .fields
                            .read(&ir::PlanBudget::default())
                            .unwrap()[*field]
                            .ty
                            .clone()
                    }
                    ir::Ty::Struct(structure) => {
                        assert!(selected.is_none());
                        structure
                            .def()
                            .fields
                            .read(&ir::PlanBudget::default())
                            .unwrap()[*field]
                            .ty
                            .clone()
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
    if let ir::MatchPattern::Alternatives { ty, rows } = pattern {
        assert_eq!(case.name, "or_boundary");
        assert_eq!(*ty, ir::Ty::Tuple(vec![ir::Ty::Bool, ir::Ty::Bool].into()));
        assert_eq!(rows.len(), 2);
        for (row, expected) in rows.iter().zip([true, false]) {
            assert_eq!(row.len(), 1);
            assert_eq!(row[0].projection, [ir::Projection::Field(0)]);
            assert_eq!(row[0].kind, ir::PatternTestKind::Bool(expected));
            assert_eq!(projected_type(ty, &row[0].projection, &[]), ir::Ty::Bool);
        }
        return rows.len();
    }
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
            kind @ (ir::PatternTestKind::Char(_)
            | ir::PatternTestKind::IntRange { .. }
            | ir::PatternTestKind::CharRange { .. }) => {
                kind.scalar_interval().unwrap().unwrap();
                assert_eq!(Some(projected), kind.scalar_ty());
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
    let _ = case;
    1
}
#[test]
fn checked_product_rows_preserve_real_domains_predicates_and_universal_ownership() {
    let mut count = 0;
    let mut decisions = 0;
    for case in cases()
        .into_iter()
        // `(ref a, _)` is an irrefutable borrowing aggregate, not a decision.
        .filter(|c| c.outcome == Outcome::Runtime && c.name != "reference_boundary")
    {
        let prepared = source::prepare_text(
            "usefulness.rs",
            case.source,
            &source::SourceOptions::default(),
        )
        .unwrap();
        let parsed =
            frontend::parse_prepared(&prepared).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let program = frontend::resolve(&parsed).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let signatures = frontend::signature_table(&program).unwrap();
        if case.name == "macro_product" {
            assert!(program.functions.iter().any(|f| f.lexical_scope.is_some()));
        }
        let mut own = 0;
        for index in 0..program.functions.len() {
            // Includes the real rigid-template CFG/ownership path, not only i32 instantiations.
            frontend::check_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} function{index}:{e}", case.name));
            let function =
                frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
            // Param-bearing template IR is not a concrete public Function domain.
            if program.functions[index].type_params.is_empty() {
                function.validate().unwrap();
            }
            for expression in &function.exprs {
                if let ir::ExprKind::Match { arms, .. } = &expression.kind {
                    for arm in arms {
                        own += check_decision(&function, &arm.pattern, &case);
                    }
                }
            }
        }
        assert!(
            own > 0,
            "{} must retain actual checked product predicates",
            case.name
        );
        decisions += own;
        count += 1;
    }
    assert_eq!(count, 22);
    assert!(decisions >= 23);
}
#[test]
fn actual_holes_and_dead_body_obligations_are_ordinary_invalid_not_capability_boundaries() {
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
            "{} ordinary source error:{message}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 6);
}
#[test]
fn closed_parser_lint_and_plain_let_boundaries_keep_distinct_stock_classifications() {
    let mut counts = (0, 0, 0);
    for case in cases().into_iter().filter(|c| {
        matches!(
            c.outcome,
            Outcome::Unsupported | Outcome::InvalidUnsupported
        )
    }) {
        let prepared = source::prepare_text(
            "usefulness.rs",
            case.source,
            &source::SourceOptions::default(),
        );
        if case.name == "subsumed_deny" {
            assert_eq!(case.outcome, Outcome::InvalidUnsupported);
            let error = match prepared {
                Err(error) => error,
                Ok(_) => panic!("semantic deny lint policy must remain Unsupported"),
            };
            assert_eq!(error.kind, source::SourceErrorKind::Unsupported);
            assert!(error.to_string().contains("deny"));
            counts.0 += 1;
            continue;
        }
        let prepared = prepared.unwrap();
        if case.name == "plainlet_boundary" {
            assert_eq!(case.outcome, Outcome::InvalidUnsupported);
            let parsed = frontend::parse_prepared(&prepared).unwrap();
            let program = frontend::resolve(&parsed).unwrap();
            let signatures = frontend::signature_table(&program).unwrap();
            let mut rejected = false;
            for index in 0..program.functions.len() {
                if let Err(error) =
                    frontend::check_function_with_signatures(&program, index, &signatures)
                {
                    assert!(error.contains("unsupported:"), "{error}");
                    assert!(
                        error.contains(
                            "refutable enum patterns in irrefutable positions are unsupported"
                        ),
                        "{error}"
                    );
                    rejected = true;
                }
            }
            assert!(rejected);
            counts.1 += 1;
        } else {
            assert_eq!(case.outcome, Outcome::Unsupported);
            let error = match frontend::parse_prepared(&prepared) {
                Err(error) => error,
                Ok(_) => panic!("{} parser boundary accepted", case.name),
            };
            assert_eq!(error.kind, source::SourceErrorKind::Unsupported);
            counts.2 += 1;
        }
    }
    assert_eq!(counts, (1, 1, 1));
}
#[test]
fn native_pattern_usefulness_workers1_8_check_build_no_compiler_fallback() {
    // jr-rust --profile emits telemetry, not optimization. The coordinator runs
    // this same target under debug and release BOOTSTRAP compiler profiles.
    // Neither run asserts a nonexistent optimized native code-generation mode.
    stock_all();
    let mut gates = 0;
    let mut nonempty_images = 0;
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
                    let stdout = case.stdout.expect("runtime source oracle");
                    runtime(&image, stdout, &scratch);
                    nonempty_images += usize::from(!stdout.is_empty());
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
    assert_eq!(gates, 128);
    assert_eq!(modes, (46, 46, 12, 12, 2, 2, 4, 4));
    eprintln!(
        "NATIVE PATTERN USEFULNESS:128 cells, modes{modes:?};{nonempty_images} nonempty-output main images;4 stock-valid Unsupported cells and8 stock-invalid/native-Unsupported cells are distinct boundaries, not ordinary-invalid parity or full Rust support"
    );
}
