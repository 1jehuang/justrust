//! Acceptance of the public checked-body boundary, without a stock compiler oracle.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use ir::{CheckedFunction, Completion, ExprId, ExprKind, IntTy, LocalId, Stmt, Ty};
use justrust_engine::compiler::{backend, frontend, ir};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let epoch = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        for _ in 0..100 {
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("jr-typed-ir-{}-{epoch}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch: {error}"),
            }
        }
        panic!("could not allocate scratch");
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let path = self.path(name);
        new_file(&path).write_all(bytes).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> fs::File {
    OpenOptions::new()
        .create_new(true)
        .write(true)
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
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Vec<u8> {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    command
        .stdin(Stdio::null())
        .stdout(new_file(&stdout))
        .stderr(new_file(&stderr));
    let mut child = Running(
        command
            .spawn()
            .unwrap_or_else(|error| panic!("spawn {command:?}: {error}")),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(seconds),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    let out = fs::read(stdout).unwrap();
    let err = fs::read(stderr).unwrap();
    assert!(
        status.success(),
        "{command:?}: {status}\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out),
        String::from_utf8_lossy(&err)
    );
    out
}
fn bounded(name: &str, run: impl FnOnce()) {
    const CHILD: &str = "JR_TYPED_IR_ACCEPTANCE_CHILD";
    if std::env::var(CHILD).as_deref() == Ok(name) {
        run();
        return;
    }
    let scratch = Scratch::new();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args(["--exact", name, "--nocapture"])
        .env(CHILD, name);
    capture(&mut child, &scratch, 30);
}
fn typed(source: &str, name: &str) -> CheckedFunction {
    let program = frontend::parse(source).unwrap();
    let signatures = frontend::signature_table(&program).unwrap();
    let index = program
        .functions
        .iter()
        .position(|f| f.name == name)
        .unwrap();
    let function = frontend::type_function_with_signatures(&program, index, &signatures).unwrap();
    function
        .validate()
        .expect("checker emitted invalid references");
    function
}
fn binding(statement: &Stmt) -> (LocalId, ExprId) {
    match statement {
        Stmt::Let { local, value } => (*local, *value),
        other => panic!("expected binding: {other:?}"),
    }
}
fn read(function: &CheckedFunction, id: ExprId) -> LocalId {
    match function.exprs[id.0].kind {
        ExprKind::Local(local) => local,
        ref other => panic!("expected local read: {other:?}"),
    }
}
fn binary(function: &CheckedFunction, id: ExprId) -> (ExprId, ExprId) {
    match function.exprs[id.0].kind {
        ExprKind::Binary { left, right, .. } => (left, right),
        ref other => panic!("expected binary: {other:?}"),
    }
}

#[test]
fn checked_ir_shadow_initializers_and_nested_scope_reads_have_resolved_ids() {
    bounded(
        "checked_ir_shadow_initializers_and_nested_scope_reads_have_resolved_ids",
        || {
            let function = typed(
                "fn inspect(x:i32)->i32 { let x=x+1; let y={let x=x+2; x}; x+y } fn main() {}",
                "inspect",
            );
            assert_eq!(function.params.len(), 1);
            let parameter = function.params[0];
            let (outer, initializer) = binding(&function.body.statements[0]);
            let (y, inner_expr) = binding(&function.body.statements[1]);
            assert_ne!(parameter, outer);
            assert_eq!(
                read(&function, binary(&function, initializer).0),
                parameter,
                "shadow initializer reads previous binding"
            );
            let ExprKind::Block(inner) = &function.exprs[inner_expr.0].kind else {
                panic!("missing nested block")
            };
            let (nested, value) = binding(&inner.statements[0]);
            assert_ne!(nested, outer);
            assert_ne!(nested, parameter);
            assert_eq!(read(&function, binary(&function, value).0), outer);
            assert_eq!(read(&function, inner.tail.unwrap()), nested);
            let (left, right) = binary(&function, function.body.tail.unwrap());
            assert_eq!(
                read(&function, left),
                outer,
                "nested shadow must not escape"
            );
            assert_eq!(read(&function, right), y);
            for local in [parameter, outer, nested] {
                assert_eq!(function.locals[local.0].name, "x");
                assert_eq!(function.locals[local.0].ty, Ty::Int(IntTy::I32));
            }
            assert_eq!(function.result, Ty::Int(IntTy::I32));
        },
    );
}

#[test]
fn checked_ir_assignments_resolve_to_the_correct_mutable_shadow() {
    bounded(
        "checked_ir_assignments_resolve_to_the_correct_mutable_shadow",
        || {
            let function = typed(
                "fn main() { let mut x=1; { let mut x=x+1; x=3; } x=4; }",
                "main",
            );
            let (outer, _) = binding(&function.body.statements[0]);
            let Stmt::Expr(block) = function.body.statements[1] else {
                panic!("missing block")
            };
            let ExprKind::Block(inner) = &function.exprs[block.0].kind else {
                panic!("missing block expression")
            };
            let (nested, _) = binding(&inner.statements[0]);
            let Stmt::Assign {
                local: inner_target,
                ..
            } = inner.statements[1]
            else {
                panic!("missing inner assignment")
            };
            let Stmt::Assign {
                local: outer_target,
                ..
            } = function.body.statements[2]
            else {
                panic!("missing outer assignment")
            };
            assert_ne!(outer, nested);
            assert_eq!(inner_target, nested);
            assert_eq!(outer_target, outer);
            assert!(function.locals[outer.0].mutable);
            assert!(function.locals[nested.0].mutable);
        },
    );
}

#[test]
fn checked_ir_inference_is_concrete_and_divergence_is_not_unit_flow() {
    bounded(
        "checked_ir_inference_is_concrete_and_divergence_is_not_unit_flow",
        || {
            let function = typed(
                "fn main() { let flag=1<2; let number=3; let unit=println!(\"unit\"); }",
                "main",
            );
            for (statement, expected) in
                function
                    .body
                    .statements
                    .iter()
                    .zip([Ty::Bool, Ty::Int(IntTy::I32), Ty::Unit])
            {
                let (local, value) = binding(statement);
                assert_eq!(function.locals[local.0].ty, expected);
                assert_eq!(function.exprs[value.0].ty, expected);
                assert_eq!(function.exprs[value.0].completion, Completion::Returns);
                assert_eq!(function.exprs[value.0].flow(), Some(expected));
            }
            assert_eq!(function.result, Ty::Unit);
            let function = typed(
                "fn choose(flag:bool)->i32 { if flag { return 1; } else { return 2; } } fn main() {}",
                "choose",
            );
            let divergent = function
                .exprs
                .iter()
                .filter(|expr| matches!(expr.kind, ExprKind::If { .. }))
                .collect::<Vec<_>>();
            assert_eq!(divergent.len(), 1);
            assert_eq!(divergent[0].completion, Completion::Never);
            assert_eq!(
                divergent[0].ty,
                Ty::Never,
                "pure divergence has actual Never type"
            );
            assert_eq!(divergent[0].flow(), None);
            assert_eq!(function.result, Ty::Int(IntTy::I32));
        },
    );
}

#[test]
fn checked_ir_calls_retain_canonical_targets_and_validated_signatures() {
    bounded(
        "checked_ir_calls_retain_canonical_targets_and_validated_signatures",
        || {
            let function = typed(
                "use crate::tools::accept as alias; mod tools { pub fn accept(flag:bool,n:i32)->bool { flag && n>0 } } fn main() { let ok=alias(true,4); }",
                "main",
            );
            let calls = function
                .exprs
                .iter()
                .filter_map(|expr| match &expr.kind {
                    ExprKind::Call {
                        name,
                        args,
                        signature,
                    } => Some((expr, name, args, signature)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            assert_eq!(calls.len(), 1);
            let (expr, name, args, signature) = calls[0];
            assert_eq!(name, "tools::accept");
            assert_eq!(signature.params, [Ty::Bool, Ty::Int(IntTy::I32)]);
            assert_eq!(signature.result, Ty::Bool);
            assert_eq!(expr.ty, Ty::Bool);
            assert_eq!(expr.completion, Completion::Returns);
            for (&arg, expected) in args.iter().zip(&signature.params) {
                assert_eq!(function.exprs[arg.0].flow(), Some(expected.clone()));
            }
        },
    );
}

#[test]
fn checked_ir_rejects_invalid_unused_bodies_without_publishing_typed_results() {
    bounded(
        "checked_ir_rejects_invalid_unused_bodies_without_publishing_typed_results",
        || {
            for body in [
                "let x:i32=true;",
                "let flag:bool=1;",
                "take(true);",
                "return true;",
            ] {
                let source = format!("fn take(x:i32) {{}} fn unused() {{ {body} }} fn main() {{}}");
                let program = frontend::parse(&source).unwrap();
                let signatures = frontend::signature_table(&program).unwrap();
                let unused = program
                    .functions
                    .iter()
                    .position(|f| f.name == "unused")
                    .unwrap();
                let error = frontend::type_function_with_signatures(&program, unused, &signatures)
                    .expect_err("invalid unused body published typed IR");
                assert!(!error.is_empty());
                let main = program
                    .functions
                    .iter()
                    .position(|f| f.name == "main")
                    .unwrap();
                frontend::type_function_with_signatures(&program, main, &signatures)
                    .unwrap()
                    .validate()
                    .unwrap();
                assert!(
                    backend::emit(&program).is_err(),
                    "compatibility emitter ignored invalid unused body"
                );
            }
            let program = frontend::parse("fn main() {}").unwrap();
            let signatures = frontend::signature_table(&program).unwrap();
            assert!(
                frontend::type_function_with_signatures(&program, usize::MAX, &signatures).is_err()
            );
        },
    );
}

#[test]
fn checked_ir_native_lowering_and_cli_preserve_shadow_calls_and_return_execution() {
    bounded(
        "checked_ir_native_lowering_and_cli_preserve_shadow_calls_and_return_execution",
        || {
            use std::os::unix::fs::PermissionsExt;
            let source = "fn choose(flag:bool,x:i32)->i32 { let x=x+1; if flag { let x=x+2; return x; } x } fn main() { let x=10; let x=x+1; let flag=choose(true,x)==14 && choose(false,x)==12; if flag { println!(\"typed native ok\"); } }";
            let program = frontend::parse(source).unwrap();
            let signatures = frontend::signature_table(&program).unwrap();
            let objects = program
                .functions
                .iter()
                .enumerate()
                .map(|(index, function)| {
                    let checked =
                        frontend::type_function_with_signatures(&program, index, &signatures)
                            .unwrap();
                    checked.validate().unwrap();
                    (
                        function.name.clone(),
                        backend::emit_checked_function(&checked).unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            let linked = backend::link(&objects).unwrap();
            let compatibility = backend::emit(&program).unwrap();
            assert_eq!(
                linked, compatibility,
                "compatibility path must use the same checked lowering"
            );
            let scratch = Scratch::new();
            for (name, bytes) in [("checked", linked), ("compatibility", compatibility)] {
                assert!(bytes.len() >= 64);
                assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                let path = scratch.write(name, &bytes);
                fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
                assert_eq!(
                    capture(&mut Command::new(path), &scratch, 5),
                    b"typed native ok\n"
                );
            }
            let input = scratch.write("input.rs", source.as_bytes());
            for workers in [1, 8] {
                for check in [true, false] {
                    let output = scratch.path(&format!("program-{workers}"));
                    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                    command
                        .arg(&input)
                        .args(["--workers", &workers.to_string()]);
                    if check {
                        command.arg("--check");
                    } else {
                        command.arg("-o").arg(&output);
                    }
                    capture(&mut command, &scratch, 20);
                    if check {
                        assert!(!output.exists());
                    } else {
                        assert_eq!(
                            capture(&mut Command::new(output), &scratch, 5),
                            b"typed native ok\n"
                        );
                    }
                }
            }
        },
    );
}

#[test]
fn checked_ir_divergence_discards_pending_temporaries_and_preserves_short_circuit() {
    bounded(
        "checked_ir_divergence_discards_pending_temporaries_and_preserves_short_circuit",
        || {
            use std::os::unix::fs::PermissionsExt;
            let source = "fn add(a:i32,b:i32)->i32 { a+b } fn binary()->i32 { 10+{return 7; 0} } fn argument()->i32 { add(3,{return 9;}) } fn condition()->i32 { while {return 11;} {} 0 } fn short()->bool { false && {return true;} } fn short_return()->bool { true && {return true;} } fn main() { if binary()+argument()+condition()==27 && !short() && short_return() { println!(\"divergence ok\"); } }";
            for name in ["binary", "argument"] {
                let function = typed(source, name);
                let tail = &function.exprs[function.body.tail.unwrap().0];
                assert_eq!(tail.completion, Completion::Never);
                assert_eq!(
                    tail.ty,
                    Ty::Int(IntTy::I32),
                    "actual operator/call result type survives divergence"
                );
                assert_eq!(tail.flow(), None);
            }
            for name in ["short", "short_return"] {
                let function = typed(source, name);
                let tail = &function.exprs[function.body.tail.unwrap().0];
                assert_eq!(tail.completion, Completion::Returns);
                assert_eq!(tail.flow(), Some(Ty::Bool));
            }
            let program = frontend::parse(source).unwrap();
            let bytes = backend::emit(&program).unwrap();
            let scratch = Scratch::new();
            let path = scratch.write("divergence", &bytes);
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
            assert_eq!(
                capture(&mut Command::new(path), &scratch, 5),
                b"divergence ok\n"
            );
        },
    );
}

#[test]
fn checked_ir_unreachable_tail_retains_its_actual_type_not_unit_placeholder() {
    bounded(
        "checked_ir_unreachable_tail_retains_its_actual_type_not_unit_placeholder",
        || {
            for (source, expected) in [
                (
                    "fn f()->i32 {{return 3; 0}} fn main(){}",
                    Ty::Int(IntTy::I32),
                ),
                ("fn f()->bool {{return true; false}} fn main(){}", Ty::Bool),
                ("fn f()->i32 {{return 3;}} fn main(){}", Ty::Never),
            ] {
                let function = typed(source, "f");
                let tail = &function.exprs[function.body.tail.unwrap().0];
                assert_eq!(tail.ty, expected);
                assert_eq!(tail.completion, Completion::Never);
                assert_eq!(tail.flow(), None);
            }
            for source in [
                "fn f()->i32 {return 3; ()} fn main(){}",
                "fn f()->i32 {1+{return 3;}} fn main(){}",
                "fn f(){let x=1=={return;};} fn main(){}",
            ] {
                let program = frontend::parse(source).unwrap();
                let signatures = frontend::signature_table(&program).unwrap();
                let index = program
                    .functions
                    .iter()
                    .position(|f| f.name == "f")
                    .unwrap();
                assert!(
                    frontend::type_function_with_signatures(&program, index, &signatures).is_err(),
                    "invalid actual type accepted: {source}"
                );
            }
        },
    );
}
