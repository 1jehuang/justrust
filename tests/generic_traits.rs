//! Type-parameterized trait applications with real checked default/bound/ownership paths.
//! No new trait-argument inference is claimed: multi-application dot remains a distinct boundary.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::{self, frontend, ir, source};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";
// Final28 stock JSON SHA256 e62a91d49ac9adf66a9d2186f7d999d52ca4b574a0c03ec86a860f3726858c09
// Original11 measured baseline source bytes/order retained unchanged.
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
            name: "concrete_ufcs",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}fn main(){if <S as Take<i32>>::take(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "rigid_forward",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl<T> Take<T> for S{fn take(self,x:T)->T{x}}fn forward<T,U:Take<T>>(u:U,x:T)->T{u.take(x)}fn main(){if forward(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "distinct_applications",
            source: r#"trait Get<T>{fn get(self)->T;}struct S;impl Get<i32> for S{fn get(self)->i32{42}}impl Get<bool> for S{fn get(self)->bool{true}}fn main(){if <S as Get<bool>>::get(S){if <S as Get<i32>>::get(S)==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "input_inferred_method",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}impl Take<bool> for S{fn take(self,x:bool)->bool{x}}fn main(){if S.take(true){if S.take(42i32)==42{println!("ok");}}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Unsupported,
        },
        Case {
            name: "ambiguous_application",
            source: r#"trait Get<T>{fn get(self)->T;}struct S;impl Get<i32> for S{fn get(self)->i32{42}}impl Get<bool> for S{fn get(self)->bool{true}}fn main(){let x=S.get();}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0283]"),
            outcome: Outcome::InvalidUnsupported,
        },
        Case {
            name: "wrong_impl_signature",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:bool)->i32{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0053]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_trait_argument",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}fn forward<U:Take>(u:U){}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0107]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_application_bound",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}fn forward<T,U:Take<T>>(u:U,x:T)->T{u.take(x)}fn main(){let x=forward(S,true);}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "universal_bad_body",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl<T> Take<T> for S{fn take(self,x:T)->T{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "universal_noncopy_move",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}fn forward<T,U:Take<T>>(u:U,x:T)->T{let y=x;u.take(x)}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "overlapping_same_application",
            source: r#"trait Get<T>{fn get(self)->T;}struct S;impl<T> Get<T> for S{fn get(self)->T{loop{}}}impl Get<i32> for S{fn get(self)->i32{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0119]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "default_method",
            source: r#"trait Take<T>:Sized{fn take(self,x:T)->T{x}}struct S;impl Take<i32> for S{}fn main(){if <S as Take<i32>>::take(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "supertrait_forward",
            source: r#"trait Parent<T>{fn take(self,x:T)->T;}trait Child<T>:Parent<T>{}struct S;impl<T> Parent<T> for S{fn take(self,x:T)->T{x}}impl<T> Child<T> for S{}fn forward<T,U:Child<T>>(u:U,x:T)->T{u.take(x)}fn main(){if forward(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "trait_parameter_bound",
            source: r#"trait Take<T:Copy>:Sized{fn take(self,x:T)->T{let y=x;x}}struct S;impl<T:Copy> Take<T> for S{}fn main(){if <S as Take<i32>>::take(S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "two_ordered_trait_arguments",
            source: r#"trait Take<T:Copy,U:Copy>{fn take(self,x:T,y:U)->T;}struct S;impl Take<i32,bool> for S{fn take(self,x:i32,y:bool)->i32{if y{x}else{0}}}fn main(){if <S as Take<i32,bool>>::take(S,42,true)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "trait_argument_only_impl_parameter",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct Carrier{n:i32}impl<T> Take<T> for Carrier{fn take(self,x:T)->T{x}}fn main(){if <Carrier as Take<i32>>::take(Carrier{n:7},42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "macro_trait_application",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}fn main(){macro_rules! take{($x:literal)=>{<S as Take<i32>>::take(S,$x)};}if take!(42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "import_trait_alias",
            source: r#"mod m{pub trait Take<T>{fn take(self,x:T)->T;}pub struct S;impl Take<i32> for S{fn take(self,x:i32)->i32{x}}}use m::Take as Alias;fn main(){if <m::S as Alias<i32>>::take(m::S,42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "universal_noncopy_forward",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;struct Cell{n:i32}impl<T> Take<T> for S{fn take(self,x:T)->T{x}}fn forward<T,U:Take<T>>(u:U,x:T)->T{u.take(x)}fn main(){let cell=forward(S,Cell{n:42});if cell.n==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "associated_function_application",
            source: r#"trait Take<T>{fn take(x:T)->T;}struct S;impl<T> Take<T> for S{fn take(x:T)->T{x}}fn main(){if <S as Take<i32>>::take(42)==42{println!("ok");}}"#,
            stdout: Some("ok\n"),
            stock_diagnostic: None,
            outcome: Outcome::Runtime,
        },
        Case {
            name: "trait_argument_bound_violation",
            source: r#"trait Take<T:Copy>{fn take(self,x:T)->T;}struct S;struct Cell{n:i32}impl Take<Cell> for S{fn take(self,x:Cell)->Cell{x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0277]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unconstrained_impl_parameter",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl<T,U> Take<T> for S{fn take(self,x:T)->T{x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0207]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "excess_trait_arity",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32,bool> for S{fn take(self,x:i32)->i32{x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0107]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_required_item",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0046]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unused_bad_default",
            source: r#"trait Take<T>:Sized{fn take(self,x:T)->T{42}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0308]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "wrong_impl_return",
            source: r#"trait Take<T>{fn take(self,x:T)->T;}struct S;impl Take<i32> for S{fn take(self,x:i32)->bool{true}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0053]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "missing_supertrait_application",
            source: r#"trait Parent<T>{fn take(self,x:T)->T;}trait Child<T>:Parent<T>{}struct S;impl Parent<bool> for S{fn take(self,x:bool)->bool{x}}impl Child<i32> for S{}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0277]"),
            outcome: Outcome::Invalid,
        },
        Case {
            name: "unused_default_noncopy_move",
            source: r#"trait Take<T>:Sized{fn take(self,x:T)->T{let y=x;x}}fn main(){}"#,
            stdout: None,
            stock_diagnostic: Some("error[E0382]"),
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
            "jr-generic-traits-{}-{nonce}-{}",
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
        "GENERIC TRAITS BASELINE:{BASELINE_STOCK}\nACTUAL STOCK:\n{}",
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
                    "--crate-name=generic_traits_oracle",
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
    assert_eq!(counts, (13, 15));
    assert_eq!(partition, (12, 14, 1, 1));
}
#[test]
fn stock_nested_generic_traits_oracle_edition2024_actual_toolchain() {
    stock_all();
}
#[test]
fn source_trait_binders_arguments_defaults_and_bounds_remain_genuine() {
    let mut count = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        let prepared =
            source::prepare_text("traits.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed =
            frontend::parse_prepared(&prepared).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let program = &parsed.program;
        assert!(!program.traits.is_empty());
        for declaration in &program.traits {
            assert!(!declaration.type_params.is_empty());
            for parameter in &declaration.type_params {
                assert!(!program.names.spelling(*parameter).unwrap().is_empty());
            }
        }
        let mut applications = 0;
        for implementation in &program.impls {
            if let Some(frontend::TraitPath::Source(path)) = &implementation.trait_path {
                assert!(path.segments.iter().any(|segment| !segment.args.is_empty()));
                applications += 1;
            }
        }
        assert!(applications > 0);
        if case.name == "trait_parameter_bound" || case.name == "two_ordered_trait_arguments" {
            assert!(
                program
                    .traits
                    .iter()
                    .any(|declaration| !declaration.bounds.is_empty())
            );
        }
        if case.name == "two_ordered_trait_arguments" {
            assert_eq!(program.traits[0].type_params.len(), 2);
        }
        if matches!(case.name, "default_method" | "trait_parameter_bound") {
            assert!(
                program
                    .traits
                    .iter()
                    .flat_map(|declaration| &declaration.items)
                    .any(|item| item.default.is_some())
            );
        }
        count += 1;
    }
    assert_eq!(count, 12);
}
#[test]
fn checked_trait_applications_keep_rigid_argument_identity_and_universal_ownership() {
    let mut count = 0;
    let mut rigid = 0;
    for case in cases()
        .into_iter()
        .filter(|c| c.outcome == Outcome::Runtime)
    {
        // Complete public pipeline checks trait defaults and unused impl methods too.
        compiler::compile_observed(case.source.to_owned(), 1, true)
            .unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let prepared =
            source::prepare_text("traits.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        let program = frontend::resolve(&parsed).unwrap_or_else(|e| panic!("{}:{e}", case.name));
        let signatures = frontend::signature_table(&program).unwrap();
        if case.name == "macro_trait_application" {
            assert!(program.functions.iter().any(|f| f.lexical_scope.is_some()));
        }
        let mut own_rigid = 0;
        for index in 0..program.functions.len() {
            frontend::check_function_with_signatures(&program, index, &signatures)
                .unwrap_or_else(|e| panic!("{} function{index}:{e}", case.name));
            let function =
                frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
            if program.functions[index].type_params.is_empty() {
                function.validate().unwrap();
            }
            for expression in &function.exprs {
                if let ir::ExprKind::Call {
                    target: ir::Callee::TraitItem(call),
                    ..
                } = &expression.kind
                {
                    assert!(
                        !call.trait_ref.args.is_empty(),
                        "{} must not erase trait application arguments",
                        case.name
                    );
                    assert!(
                        call.method_args.is_empty(),
                        "trait arguments are not method arguments"
                    );
                    if matches!(call.trait_ref.self_ty, ir::Ty::Param(_)) {
                        assert_eq!(call.trait_ref.args.len(), 1);
                        assert!(matches!(call.trait_ref.args[0], ir::Ty::Param(_)));
                        own_rigid += 1;
                    }
                }
            }
        }
        if matches!(
            case.name,
            "rigid_forward" | "supertrait_forward" | "universal_noncopy_forward"
        ) {
            assert!(
                own_rigid > 0,
                "{} lost real rigid trait-argument proof transport",
                case.name
            );
            rigid += 1;
        }
        count += 1;
    }
    assert_eq!(count, 12);
    assert_eq!(rigid, 3);
}
#[test]
fn bad_application_arity_signatures_bounds_overlap_and_unused_generic_bodies_are_invalid() {
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
            "{} ordinary error must not become Unsupported:{message}",
            case.name
        );
        count += 1;
    }
    assert_eq!(count, 14);
}
#[test]
fn multiple_trait_applications_do_not_claim_new_dot_argument_inference() {
    let mut counts = (0, 0);
    for case in cases().into_iter().filter(|c| {
        matches!(
            c.outcome,
            Outcome::Unsupported | Outcome::InvalidUnsupported
        )
    }) {
        let prepared =
            source::prepare_text("traits.rs", case.source, &source::SourceOptions::default())
                .unwrap();
        let parsed = frontend::parse_prepared(&prepared).unwrap();
        frontend::resolve(&parsed).unwrap();
        let error = match compiler::compile_observed(case.source.to_owned(), 1, true) {
            Err(error) => error,
            Ok(_) => panic!("{} unadmitted trait-argument inference accepted", case.name),
        };
        let message = error.to_string();
        assert!(message.contains("unsupported:"), "{}:{message}", case.name);
        assert!(
            message.contains("multiple trait applications require explicit UFCS; trait argument inference is unsupported"),
            "{}:{message}",
            case.name
        );
        match case.outcome {
            Outcome::Unsupported => {
                assert!(case.stdout.is_some());
                assert!(case.stock_diagnostic.is_none());
                counts.0 += 1;
            }
            Outcome::InvalidUnsupported => {
                assert!(case.stdout.is_none());
                assert_eq!(case.stock_diagnostic, Some("error[E0283]"));
                counts.1 += 1;
            }
            _ => unreachable!(),
        }
    }
    assert_eq!(counts, (1, 1));
}
#[test]
fn native_generic_traits_workers1_8_check_build_no_compiler_fallback() {
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
    assert_eq!(gates, 112);
    assert_eq!(modes, (24, 24, 28, 28, 2, 2, 2, 2));
    eprintln!(
        "NATIVE GENERIC TRAITS:112 cells, modes{modes:?};24 nonempty-output main images;4 stock-valid Unsupported cells and4 stock-invalid/native-Unsupported cells are distinct boundaries, not ordinary-invalid parity or full Rust support"
    );
}
