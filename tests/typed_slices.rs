//! General typed shared slices with exact element identity, Copy and loan authority.
//! Native private descriptor/layout tests do not claim Rust ABI or arbitrary borrow support.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Stock32 JSON SHA256 7fdc2d02f5053be00f5724b3239634915e23afda97767b4b1c43994db0017bc4.
// Native codegen optimization is not selected by jr-rust --profile (telemetry).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    Runtime,
    Trap,
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
            name: "typed_shared_slice",
            source: r###"fn sum(s:&[i32])->i32{s[0]+s[1]}fn main(){let a=[-3,45];if sum(&a)==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "wide_shared_slice",
            source: r###"fn read(s:&[i128])->i128{s[1]}fn main(){let a=[1i128,-170141183460469231731687303715884105728i128];if read(&a)==-170141183460469231731687303715884105728i128{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_copy_shared_slice",
            source: r###"fn read<T:Copy>(s:&[T])->T{s[0]}fn main(){let a=[42i32];if read(&a)==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "unsigned_wide",
            source: r###"fn read(s:&[u128])->u128{s[1]}fn main(){let a=[0u128,340282366920938463463374607431768211455u128];if read(&a)==340282366920938463463374607431768211455u128{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "float32",
            source: r###"fn read(s:&[f32])->f32{s[1]}fn main(){let a=[1.0f32,42.5f32];if read(&a)==42.5f32{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "float64",
            source: r###"fn read(s:&[f64])->f64{s[0]}fn main(){let a=[-42.5f64];if read(&a)==-42.5f64{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "bool",
            source: r###"fn read(s:&[bool])->bool{s[1]}fn main(){let a=[false,true];if read(&a){println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "signed_narrow",
            source: r###"fn read(s:&[i8])->i8{s[0]}fn main(){let a=[-128i8,127i8];if read(&a)==-128i8{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "byte_regression",
            source: r###"fn read(s:&[u8])->u8{s[1]}fn main(){let a=[1u8,255u8];if read(&a)==255u8{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "copy_record",
            source: r###"struct Cell{n:i32,wide:i128}impl Clone for Cell{fn clone(&self)->Self{Cell{n:self.n,wide:self.wide}}}impl Copy for Cell{}fn read(s:&[Cell])->Cell{s[0]}fn main(){let a=[Cell{n:42,wide:12345678901234567890i128}];let x=read(&a);if x.n==42{if x.wide==12345678901234567890i128{if a[0].n==42{println!("ok");}}}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "copy_tuple",
            source: r###"fn read(s:&[(i32,bool)])->(i32,bool){s[1]}fn main(){let a=[(1,false),(42,true)];let x=read(&a);if x.1{if x.0==42{println!("ok");}}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "copy_array",
            source: r###"fn read(s:&[[i32;2]])->[i32;2]{s[1]}fn main(){let a=[[1,2],[40,2]];let x=read(&a);if x[0]+x[1]==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_two_instances",
            source: r###"fn read<T:Copy>(s:&[T])->T{s[0]}fn main(){let a=[42i32];let b=[true];if read(&b){if read(&a)==42{println!("ok");}}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_copy_aggregate",
            source: r###"struct Cell{n:i32,wide:i128}impl Clone for Cell{fn clone(&self)->Self{Cell{n:self.n,wide:self.wide}}}impl Copy for Cell{}fn read<T:Copy>(s:&[T])->T{s[0]}fn main(){let a=[Cell{n:42,wide:7i128}];let x=read(&a);if x.n==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "noncopy_len",
            source: r###"struct Cell{n:i32}fn length(s:&[Cell])->usize{s.len()}fn main(){let a=[Cell{n:42}];if length(&a)==1usize{if a[0].n==42{println!("ok");}}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "descriptor_copy",
            source: r###"fn read(s:&[i32])->i32{let other=s;other[1]+s[0]}fn main(){let a=[40,2];let s:&[i32]=&a;if read(s)==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "index_once",
            source: r###"fn index()->usize{println!("index");1usize}fn read(s:&[i32])->i32{s[index()]}fn main(){let a=[0,42];if read(&a)==42{println!("ok");}}"###,
            stdout: Some("index\nok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_and_import",
            source: r###"mod m{pub fn read(s:&[i32])->i32{s[0]}}use m::read as get;fn main(){macro_rules! run{($a:ident)=>{get(&$a)};}let a=[42];if run!(a)==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "bounds_empty",
            source: r###"fn read(s:&[i32])->i32{s[0]}fn main(){let a:[i32;0]=[];let x=read(&a);}"###,
            stdout: Some(""),
            stock_diagnostic: Some("index out of bounds"),
            outcome: Outcome::Trap,
        },
        Case {
            name: "bounds_equal_len",
            source: r###"fn read(s:&[i32])->i32{s[s.len()]}fn main(){let a=[42];let x=read(&a);}"###,
            stdout: Some(""),
            stock_diagnostic: Some("index out of bounds"),
            outcome: Outcome::Trap,
        },
        Case {
            name: "bounds_max_index",
            source: r###"fn read(s:&[i32])->i32{s[18446744073709551615usize]}fn main(){let a=[42];let x=read(&a);}"###,
            stdout: Some(""),
            stock_diagnostic: Some("index out of bounds"),
            outcome: Outcome::Trap,
        },
        Case {
            name: "element_mismatch",
            source: r###"fn read(s:&[i32])->i32{s[0]}fn main(){let a=[true];let x=read(&a);}"###,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "noncopy_extraction",
            source: r###"struct Cell{n:i32}fn read(s:&[Cell])->Cell{s[0]}fn main(){}"###,
            stdout: None,
            stock_diagnostic: Some("error[E0508]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "generic_without_copy",
            source: r###"fn read<T>(s:&[T])->T{s[0]}fn main(){}"###,
            stdout: None,
            stock_diagnostic: Some("error[E0508]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "live_array_loan",
            source: r###"fn main(){let mut a=[1i32,2];let s:&[i32]=&a;a[0]=3;let x=s[0];}"###,
            stdout: None,
            stock_diagnostic: Some("error[E0506]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_index_type",
            source: r###"fn read(s:&[i32])->i32{s[true]}fn main(){}"###,
            stdout: None,
            stock_diagnostic: Some("error[E0277]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "noncopy_shared_field",
            source: r###"struct Cell{n:i32}fn read(s:&[Cell])->i32{s[0].n}fn main(){let a=[Cell{n:42}];let n=read(&a);if n+a[0].n==84{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "reference_elements",
            source: r###"fn read(s:&[&i32])->i32{*s[0]}fn main(){let x=42;let a=[&x];if read(&a)==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "slice_return",
            source: r###"fn read(s:&[i32])->&[i32]{s}fn main(){let a=[42];if read(&a)[0]==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "mutable_slice",
            source: r###"fn read(s:&mut [i32])->i32{s[0]=42;s[0]}fn main(){let mut a=[1];if read(&mut a)==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            // Promoted unchanged: shared subslices are supported (SUBSLICE.md).
            name: "slice_range",
            source: r###"fn main(){let a=[1,42];let s:&[i32]=&a[1..];if s[0]==42{println!("ok");}}"###,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "generic_reference_substitution",
            source: r###"fn read<T:Copy>(s:&[T])->T{s[0]}fn main(){let x=42;let a=[&x];if *read(&a)==42{println!("ok");}}"###,
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
            "jr-typed-slices-{}-{nonce}-{}",
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
        code: status
            .code()
            .unwrap_or_else(|| -status.signal().expect("Unix signal")),
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
        "TYPED SLICES BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = [0usize; 4];
    for case in cases() {
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=typed_slice_oracle",
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
        match case.outcome {
            Outcome::Invalid => {
                counts[2] += 1;
                assert_eq!(result.code, 1, "{}:{result:?}", case.name);
                assert!(
                    String::from_utf8_lossy(&result.stderr)
                        .contains(case.stock_diagnostic.unwrap()),
                    "{}:{result:?}",
                    case.name
                );
                assert!(!image.exists());
            }
            Outcome::Trap => {
                counts[1] += 1;
                assert_eq!(result.code, 0, "{}:{result:?}", case.name);
                let bytes = fs::read(&image).unwrap();
                assert!(bytes.len() >= 64);
                assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                let run = capture(&mut Command::new(&image), &scratch, 5);
                assert_eq!(run.code, -6, "{} stock -Cpanic=abort:{run:?}", case.name);
                assert!(run.stdout.is_empty());
                assert!(
                    String::from_utf8_lossy(&run.stderr).contains("index out of bounds"),
                    "{}:{run:?}",
                    case.name
                );
            }
            Outcome::Runtime | Outcome::Unsupported => {
                counts[if case.outcome == Outcome::Runtime {
                    0
                } else {
                    3
                }] += 1;
                assert_eq!(result.code, 0, "{}:{result:?}", case.name);
                assert!(case.stock_diagnostic.is_none());
                runtime(&image, case.stdout.unwrap(), &scratch);
            }
        }
    }
    assert_eq!(counts, [21, 3, 5, 3]);
}
#[test]
fn stock_typed_slices_edition2024_actual_toolchain() {
    stock_all();
}

#[test]
fn general_slice_shape_helpers_preserve_byte_only_helpers() {
    use std::sync::Arc;
    for element in [
        ir::Ty::Int(ir::IntTy::I32),
        ir::Ty::Int(ir::IntTy::I128),
        ir::Ty::Bool,
        ir::Ty::Int(ir::IntTy::U8),
    ] {
        let slice = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Slice {
                element: Arc::new(element.clone()),
            }),
        };
        let array = ir::Ty::Ref {
            kind: ir::BorrowKind::Shared,
            referent: Arc::new(ir::Ty::Array {
                element: Arc::new(element.clone()),
                len: 3,
            }),
        };
        assert_eq!(slice.shared_slice_element(), Some(&element));
        assert_eq!(array.shared_array_element_len(), Some((&element, 3)));
        assert_eq!(
            slice.shared_byte_slice(),
            element == ir::Ty::Int(ir::IntTy::U8)
        );
        assert_eq!(
            array.shared_byte_array_len(),
            if element == ir::Ty::Int(ir::IntTy::U8) {
                Some(3)
            } else {
                None
            }
        );
        assert!(slice.shared_array_element_len().is_none());
        assert!(array.shared_slice_element().is_none());
    }
    assert!(ir::Ty::Bool.shared_slice_element().is_none());
    assert!(
        ir::Ty::Slice {
            element: Arc::new(ir::Ty::Bool)
        }
        .shared_slice_element()
        .is_none()
    );
}

#[test]
fn every_admitted_function_checks_universally_and_slice_ir_keeps_exact_element() {
    let mut count = 0;
    let mut generic = 0;
    let mut indices = 0;
    let mut coercions = 0;
    let mut lengths = 0;
    for case in cases()
        .into_iter()
        .filter(|c| matches!(c.outcome, Outcome::Runtime | Outcome::Trap))
    {
        compiler::compile_observed(case.source.to_owned(), 1, true)
            .unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let prepared =
            source::prepare_text("slices.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let signatures = frontend::signature_table(&program).unwrap();
        let plan = signatures.plan().unwrap();
        let mut own_index = 0;
        for i in 0..program.functions.len() {
            frontend::check_function_with_signatures(&program, i, &signatures)
                .unwrap_or_else(|e| panic!("{} function{i}:{e}", case.name));
            let function =
                frontend::type_function_with_signatures(&program, i, &signatures).unwrap();
            // The same immutable source world owns both the typed body and its
            // Copy/binder facts. Concrete fallback facts erase declared Copy.
            let facts = plan.callable_facts(plan.callable_decl(i).unwrap()).unwrap();
            function
                .validate_with_facts(ir::FactsCtx::Validated(&facts))
                .unwrap_or_else(|e| panic!("{} function{i}:{e}", case.name));
            if !program.functions[i].type_params.is_empty() {
                generic += 1;
            }
            for expression in &function.exprs {
                match &expression.kind {
                    ir::ExprKind::SliceIndex { slice, index } => {
                        let element = function.exprs[slice.0]
                            .ty
                            .shared_slice_element()
                            .expect("shared exact slice operand");
                        assert_eq!(&expression.ty, element, "{} exact read element", case.name);
                        assert_eq!(function.exprs[index.0].ty, ir::Ty::Int(ir::IntTy::Usize));
                        indices += 1;
                        own_index += 1;
                    }
                    ir::ExprKind::ArrayToSlice { reference } => {
                        let (kind, element, _len) = function.exprs[reference.0]
                            .ty
                            .array_ref_element_len()
                            .expect("real array reference origin");
                        assert_eq!(
                            expression.ty.slice_ref_element(),
                            Some((kind, element)),
                            "{} no reinterpretation",
                            case.name
                        );
                        coercions += 1;
                    }
                    ir::ExprKind::SliceLen { slice } => {
                        assert!(function.exprs[slice.0].ty.shared_slice_element().is_some());
                        assert_eq!(expression.ty, ir::Ty::Int(ir::IntTy::Usize));
                        lengths += 1;
                    }
                    _ => {}
                }
            }
        }
        if case.name == "noncopy_len" {
            assert_eq!(own_index, 0, "len must not extract a nonCopy element");
        } else {
            assert!(own_index > 0, "{} must use genuine SliceIndex", case.name);
        }
        count += 1;
    }
    assert_eq!(count, 24);
    assert_eq!(generic, 3);
    assert!(indices >= 20);
    assert!(coercions >= 21);
    assert!(lengths >= 2);
}

#[test]
fn wrong_elements_index_types_copy_obligations_and_live_loans_are_ordinary_invalid() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Invalid)
    {
        let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(e) => e,
            Ok(_) => panic!("{} invalid accepted", case.name),
        };
        let message = error.to_string();
        assert!(!message.is_empty());
        assert!(
            !message.contains("unsupported:"),
            "{} ordinary invalid not boundary:{message}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 5);
}
#[test]
fn valid_borrowed_projection_reference_elements_returns_mutability_and_ranges_stay_explicit_boundaries()
 {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Unsupported)
    {
        assert!(case.stdout.is_some());
        assert!(case.stock_diagnostic.is_none());
        let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(e) => e,
            Ok(_) => panic!("{} closed capability accepted", case.name),
        };
        let message = error.to_string();
        let expected = match case.name {
            "reference_elements" => {
                "slice elements containing references or slices, or nested slice carriers"
            }
            // Reference generic arguments are admitted (G3), so concrete
            // substitution now reaches the reference-element slice boundary.
            "generic_reference_substitution" => {
                "slice elements containing references or slices, or nested slice carriers"
            }
            _ => "unsupported:",
        };
        assert!(
            message.contains(expected),
            "{} explicit stock-valid capability boundary:{message}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 3);
}
#[test]
fn native_typed_slices_workers1_8_check_build_no_compiler_fallback() {
    stock_all();
    let mut gates = 0;
    let mut modes = [0usize; 8];
    let mut images = 0;
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
                let accepted = matches!(case.outcome, Outcome::Runtime | Outcome::Trap);
                assert!(
                    !diagnostic.contains("panicked at"),
                    "{}:{diagnostic}",
                    case.name
                );
                assert_eq!(
                    result.code,
                    i32::from(!accepted),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if !accepted {
                    assert!(result.stdout.is_empty());
                    assert!(!diagnostic.is_empty());
                    if case.name == "reference_elements"
                        || case.name == "generic_reference_substitution"
                    {
                        assert_eq!(case.outcome, Outcome::Unsupported);
                        assert!(
                            diagnostic.contains(
                                "slice elements containing references or slices, or nested slice carriers"
                            ),
                            "{}:{diagnostic}",
                            case.name
                        );
                    } else {
                        assert_eq!(
                            diagnostic.contains("unsupported:"),
                            case.outcome == Outcome::Unsupported,
                            "{}:{diagnostic}",
                            case.name
                        );
                    }
                }
                if !check && accepted {
                    if case.outcome == Outcome::Runtime {
                        runtime(&image, case.stdout.unwrap(), &scratch);
                    } else {
                        let bytes = fs::read(&image).unwrap();
                        assert!(bytes.len() >= 64);
                        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                        let run = capture(&mut Command::new(&image), &scratch, 5);
                        assert_eq!(run.code, -6, "{} source bounds abort:{run:?}", case.name);
                        assert!(run.stdout.is_empty());
                        assert!(
                            String::from_utf8_lossy(&run.stderr).contains("index out of bounds")
                        );
                    }
                    images += 1;
                } else {
                    for artifact in ["native", "client", "a.out"] {
                        assert!(
                            !scratch.path(artifact).exists(),
                            "{} artifact{artifact}",
                            case.name
                        );
                    }
                }
                let class = match case.outcome {
                    Outcome::Runtime => 0,
                    Outcome::Trap => 1,
                    Outcome::Invalid => 2,
                    Outcome::Unsupported => 3,
                };
                modes[class * 2 + usize::from(!check)] += 1;
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 128);
    assert_eq!(modes, [42, 42, 6, 6, 10, 10, 6, 6]);
    assert_eq!(images, 48);
    eprintln!(
        "TYPED SLICES native128cells/48ELF executions/profile, modes{modes:?};3stock-validUnsupported capabilities are not ordinary-invalid parity, general borrowing or full Rust"
    );
}
