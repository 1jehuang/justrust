//! Existing-syntax source parity and checked AST owner mutation controls.
//! Full hygiene, source macro expansion, and full Rust support remain out of scope.
//! Stock subprocesses are an independent oracle, never a native fallback.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler::frontend::{
    self, BindingName, Expr, FieldName, PlanCtx, Program, Type,
};
use justrust_engine::compiler::ir::PlanBudget;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BASELINE_STOCK: &str = "rustc 1.98.1;48a229ceaefd4985c50990b14116b6d856af0985;2026-09-01;x86_64-unknown-linux-gnu;LLVM22.1.8";

#[test]
fn resolver_module_facts_read_only_exact_namespace_snapshot() {
    // 148 is the production RESOLVE_MODULE kind, not a synthetic fixture.
    // The prepared source also contains fixed native-prelude modules.
    let mut expected_modules = None;
    for functions in [1, 8] {
        let mut source = String::from("mod a{pub type Number=i32;");
        for index in 0..functions {
            source.push_str(&format!("pub fn f{index}()->Number{{2}}"));
        }
        source.push_str("}mod b{pub fn run()->i32{crate::a::f0()}}fn main(){b::run();}");
        for workers in [1, 8] {
            for check in [true, false] {
                let result =
                    justrust_engine::compiler::compile(source.clone(), workers, check).unwrap();
                let report = &result.report;
                assert!(report.capture_complete);
                let modules = report
                    .node_evaluations
                    .iter()
                    .zip(&report.node_ids)
                    .filter(|((key, _), _)| key.kind == 148)
                    .collect::<Vec<_>>();
                let expected = *expected_modules.get_or_insert(modules.len());
                assert!(expected >= 3);
                assert_eq!(
                    modules.len(),
                    expected,
                    "function growth added module nodes"
                );
                for ((key, evaluations), node) in modules {
                    assert_eq!(*evaluations, 1);
                    assert_eq!(key.params.len(), 1);
                    let tasks = report
                        .events
                        .iter()
                        .filter(|event| event.node == Some(*node))
                        .filter_map(|event| event.task)
                        .collect::<std::collections::HashSet<_>>();
                    let producers = report
                        .dependency_edges
                        .iter()
                        .filter(|(task, _, _)| tasks.contains(task))
                        .map(|(_, producer, _)| *producer)
                        .collect::<std::collections::HashSet<_>>();
                    assert_eq!(
                        producers.len(),
                        1,
                        "module fact acquired unrelated item dependencies"
                    );
                }
                assert_eq!(result.executable.is_none(), check);
            }
        }
    }
}

struct Case {
    name: &'static str,
    source: &'static str,
    stdout: Option<&'static str>,
}
fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "local_shadows_import",
            source: "mod api {pub fn f()->i32{7}} use crate::api::{f};fn main(){let f=3;if f==3{println!(\"imports ok\");}}",
            stdout: Some("imports ok\n"),
        },
        Case {
            name: "separate_type_value",
            source: "mod types {pub struct Same{pub x:i32}} mod values {pub fn Same()->i32{7}} use crate::{types::{Same},values::{Same}};fn main(){let s=Same{x:Same()};if s.x==7{println!(\"imports ok\");}}",
            stdout: Some("imports ok\n"),
        },
        Case {
            name: "public_reexport_private_module",
            source: "mod hidden{pub mod inner{pub fn f()->i32{7}}}pub use hidden::inner as api;fn main(){if api::f()==7{println!(\"reexports ok\");}}",
            stdout: Some("reexports ok\n"),
        },
        Case {
            name: "restricted_ancestor_alias",
            source: "mod a{mod b{pub(in crate::a) fn f()->i32{7}}pub(in crate::a) use b::f as g;pub fn run()->i32{g()}}fn main(){if a::run()==7{println!(\"visibility ok\");}}",
            stdout: Some("visibility ok\n"),
        },
        Case {
            name: "private_function",
            source: "mod api{fn secret()->i32{7}}use crate::api::{secret};fn main(){secret();}",
            stdout: None,
        },
        Case {
            name: "private_field",
            source: "mod a{pub mod b{pub struct S{pub(super) x:i32}pub fn make()->S{S{x:7}}}}fn main(){let mut s=a::b::make();s.x=8;}",
            stdout: None,
        },
        Case {
            name: "shadow_alias_not_callable",
            source: "mod a{pub fn f(){}} use a::f as alias; fn main(){let alias=1;alias();}",
            stdout: None,
        },
        Case {
            name: "unused_child_no_parent_scope",
            source: "fn f(){} mod a{pub fn g(){f();}} fn main(){}",
            stdout: None,
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
            "jr-semantic-resolution-{}-{nonce}-{}",
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
        "RESOLUTION BASELINE:{BASELINE_STOCK}\nACTUALSTOCK:\n{}",
        String::from_utf8_lossy(&version.stdout)
    );
    let mut counts = (0, 0);
    for case in cases() {
        let scratch = Scratch::new();
        scratch.write("client.rs", case.source.as_bytes());
        let image = scratch.path("stock");
        let result = capture(
            Command::new(&rustc)
                .env_remove("RUSTC_BOOTSTRAP")
                .args([
                    "--edition=2024",
                    "--crate-name=semantic_resolution_oracle",
                    "--color=never",
                    "-Cpanic=abort",
                ])
                .arg(scratch.path("client.rs"))
                .arg("-o")
                .arg(&image),
            &scratch,
            20,
        );
        if let Some(expected) = case.stdout {
            counts.0 += 1;
            assert_eq!(result.code, 0, "{}:{result:?}", case.name);
            runtime(&image, expected, &scratch);
        } else {
            counts.1 += 1;
            assert_eq!(result.code, 1, "{}:{result:?}", case.name);
            assert!(!result.stderr.is_empty());
            assert!(!image.exists());
        }
    }
    assert_eq!(counts, (4, 4));
}
#[test]
fn stock_existing_resolution_corpus_edition2024() {
    stock_all();
}
#[test]
fn native_existing_resolution_workers1_8_check_build_no_compiler_fallback() {
    stock_all();
    let mut gates = 0;
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
                    i32::from(case.stdout.is_none()),
                    "{} workers{workers} check{check}:{result:?}",
                    case.name
                );
                if case.stdout.is_none() {
                    assert!(result.stdout.is_empty());
                    assert!(!diagnostic.is_empty());
                    assert!(
                        !diagnostic.contains("unsupported"),
                        "{} invalid classification:{diagnostic}",
                        case.name
                    );
                }
                if let (false, Some(expected)) = (check, case.stdout) {
                    runtime(&image, expected, &scratch);
                } else {
                    for path in ["native", "client", "a.out"] {
                        assert!(
                            !scratch.path(path).exists(),
                            "artifact {path} {}",
                            case.name
                        );
                    }
                }
                gates += 1;
            }
        }
    }
    assert_eq!(gates, 32);
}

const OWNER_SOURCE: &str = "struct Item{x:i32}fn unused<T>(param:T)->T{param}fn main(){}";
fn owner_pair() -> (Program, Program) {
    (
        frontend::parse(OWNER_SOURCE).unwrap(),
        frontend::parse(OWNER_SOURCE).unwrap(),
    )
}
fn reject_before_authority(program: Program) {
    let budget = PlanBudget::default();
    let error = match PlanCtx::from_shared_program(Arc::new(program), &budget) {
        Err(error) => error,
        Ok(_) => panic!("foreign public AST must reject"),
    };
    assert!(
        error.to_lowercase().contains("foreign") || error.to_lowercase().contains("contract"),
        "wrong rejection category:{error}"
    );
    assert!(
        !error.to_lowercase().contains("unsupported"),
        "owner violation is not a language boundary:{error}"
    );
    let usage = budget.usage_snapshot();
    assert_eq!(
        usage.applications, 0,
        "foreign names cannot publish nominal applications"
    );
    assert_eq!(
        usage.obligations, 0,
        "foreign names cannot publish obligations"
    );
    assert!(
        !usage.poisoned,
        "ordinary owner mismatch is not resource poison"
    );
}

#[test]
fn cloned_program_retains_exact_name_and_module_owners_and_plans() {
    let program = frontend::parse(OWNER_SOURCE).unwrap();
    let cloned = program.clone();
    assert!(Arc::ptr_eq(&program.names, &cloned.names));
    assert!(Arc::ptr_eq(&program.modules, &cloned.modules));
    assert!(Arc::ptr_eq(&program.symbols, &cloned.symbols));
    drop(program);
    let budget = PlanBudget::default();
    PlanCtx::from_shared_program(Arc::new(cloned), &budget).unwrap();
}
#[test]
fn foreign_program_symbols_reject_even_empty_program_before_optional_key_use() {
    let original = frontend::parse_unresolved("").unwrap().program;
    let cloned = original.clone();
    assert!(Arc::ptr_eq(&original.names, &cloned.names));
    assert!(Arc::ptr_eq(&original.modules, &cloned.modules));
    assert!(Arc::ptr_eq(&original.symbols, &cloned.symbols));
    assert!(cloned.functions.is_empty() && cloned.structs.is_empty() && cloned.traits.is_empty());
    let error = match PlanCtx::from_shared_program(Arc::new(cloned), &PlanBudget::default()) {
        Err(error) => error,
        Ok(_) => panic!("same-owner empty program must reach the existing entry-point requirement"),
    };
    assert_eq!(error, "main must exist, take no arguments and return ()");
    let foreign = frontend::parse_unresolved("").unwrap().program;
    assert!(!Arc::ptr_eq(&original.symbols, &foreign.symbols));
    let mut replaced = original;
    replaced.symbols = foreign.symbols.clone();
    reject_before_authority(replaced);
    let (mut original, foreign) = owner_pair();
    original.symbols = foreign.symbols.clone();
    reject_before_authority(original);
}

#[test]
fn replacing_public_program_name_or_module_owner_cannot_rebind_same_spelling() {
    let (mut original, foreign) = owner_pair();
    original.names = foreign.names.clone();
    reject_before_authority(original);
    let (mut original, foreign) = owner_pair();
    original.modules = foreign.modules.clone();
    reject_before_authority(original);
}
#[test]
fn foreign_unused_callable_path_fails_even_when_text_and_shape_match() {
    let (mut original, foreign) = owner_pair();
    original.functions[0].name = foreign.functions[0].name.clone();
    reject_before_authority(original);
}
#[test]
fn foreign_unused_identifier_expression_and_parameter_binding_fail_closed() {
    let (mut original, foreign) = owner_pair();
    let Some(tail) = foreign.functions[0].body.tail.as_ref() else {
        panic!("fixture tail");
    };
    assert!(matches!(tail.as_ref(), Expr::Var(_)));
    original.functions[0].body.tail = Some(tail.clone());
    reject_before_authority(original);
    let (mut original, foreign) = owner_pair();
    assert!(matches!(
        foreign.functions[0].params[0].0,
        BindingName::Named(_)
    ));
    original.functions[0].params[0].0 = foreign.functions[0].params[0].0;
    reject_before_authority(original);
}
#[test]
fn foreign_unused_type_and_generic_parameter_fail_before_nominal_authority() {
    let (mut original, foreign) = owner_pair();
    assert!(matches!(foreign.functions[0].params[0].1, Type::Param(_)));
    original.functions[0].params[0].1 = foreign.functions[0].params[0].1.clone();
    reject_before_authority(original);
    let (mut original, foreign) = owner_pair();
    original.functions[0].type_params[0] = foreign.functions[0].type_params[0];
    reject_before_authority(original);
}
#[test]
fn foreign_unused_field_identifier_and_type_path_fail_schema_admission() {
    let (mut original, foreign) = owner_pair();
    assert!(matches!(
        foreign.structs[0].fields[0].name,
        FieldName::Named(_)
    ));
    original.structs[0].fields[0].name = foreign.structs[0].fields[0].name;
    reject_before_authority(original);
    let (mut original, foreign) = owner_pair();
    original.structs[0].fields[0].ty = Type::Named(foreign.structs[0].name.clone());
    reject_before_authority(original);
}
