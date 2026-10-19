//! Mutable slices: invalid and legal-unsupported sources must
//! fail at CHECK and BUILD (workers 1 and 8) with no executable, including
//! when the offending function is unused.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
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
            let path = base.join(format!(
                "jr-mutable-slices-authority-{}-{epoch}-{id}",
                std::process::id()
            ));
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
    fn source(&self, source: &str) -> PathBuf {
        let path = self.path("input.rs");
        new_file(&path).write_all(source.as_bytes()).unwrap();
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
#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    #[allow(dead_code)]
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
fn capture(command: &mut Command, scratch: &Scratch, seconds: u64) -> Captured {
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
            "timed out: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    Captured {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn compiler(input: &Path, output: &Path, workers: usize, check: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command.arg(input).args(["--workers", &workers.to_string()]);
    if check {
        command.arg("--check");
    } else {
        command.arg("-o").arg(output);
    }
    command
}
fn rejects(label: &str, source: &str, needle: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 90);
            let stderr = String::from_utf8_lossy(&result.stderr);
            assert!(
                !result.status.success() && result.status.code().is_some(),
                "{label}: workers={workers} check={check} {result:?}"
            );
            assert!(!stderr.contains("panicked at"), "{label}: {stderr}");
            assert!(
                stderr.contains(needle),
                "{label}: expected {needle:?}: {stderr}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

#[test]
fn invalid_mutable_slice_programs_never_publish() {
    for (label, source) in [
        ("shared_write", "fn bad(s:&[i32]) { s[0]=4; } fn main(){}"),
        (
            "alias_backing",
            "fn main(){let mut a=[1,2];let r:&mut[i32]=&mut a;a[0]=9;let n=r.len();println!(\"{}\",n);}",
        ),
        (
            "two_mutable",
            "fn main(){let mut a=[1,2];let r:&mut[i32]=&mut a;let q:&mut[i32]=&mut a;let n=r.len()+q.len();println!(\"{}\",n);}",
        ),
        (
            "moved_handle",
            "fn main(){let mut a=[1,2];let r:&mut[i32]=&mut a;let q=r;let n=r.len()+q.len();println!(\"{}\",n);}",
        ),
        (
            "dangling",
            "fn main(){let r:&mut[i32];{let mut a=[1,2];r=&mut a;}let n=r.len();println!(\"{}\",n);}",
        ),
        (
            "wrong_element",
            "fn bad(s:&mut[i32]){s[0]=true;}fn main(){}",
        ),
        ("wrong_index", "fn bad(s:&mut[i32]){s[true]=2;}fn main(){}"),
    ] {
        rejects(label, source, "");
    }
}

#[test]
fn explicit_remaining_boundaries() {
    for (label, source) in [
        (
            "mutable_range",
            "fn main(){let mut a=[1,2];let r=&mut a[1..];r[0]=4;}",
        ),
        (
            "slice_return",
            "fn same(s:&mut[i32])->&mut[i32]{s}fn main(){}",
        ),
        (
            "noncopy_write",
            "struct A{x:i32} fn bad(s:&mut[A]){s[0]=A{x:2};}fn main(){}",
        ),
    ] {
        rejects(label, source, "unsupported");
    }
}

#[test]
fn forged_shared_carriers_cannot_authorize_mutation_or_unsizing() {
    use justrust_engine::compiler::{cfg, frontend, ir};
    use std::sync::Arc;
    let program = frontend::parse("fn put(s:&mut[i32]){s[0]=7;}fn unsize(){let mut a=[1];let s:&mut[i32]=&mut a;s[0]=2;}fn main(){}").unwrap();
    let budget = ir::PlanBudget::default();
    let plan = frontend::PlanCtx::from_program(&program, &budget).unwrap();
    for name in ["put", "unsize"] {
        let decl = (0..plan.callable_count())
            .map(|i| plan.callable_decl(i).unwrap())
            .find(|d| {
                let f = plan.callable_body(&program, *d).unwrap();
                f.name.len() == 1 && program.names.spelling(f.name.segments()[0]).unwrap() == name
            })
            .unwrap();
        let checked = frontend::type_callable_with_plan(&program, decl, &plan, &budget).unwrap();
        let facts = plan.callable_facts(decl).unwrap();
        checked
            .validate_with_facts(ir::FactsCtx::Validated(&facts))
            .unwrap();
        if name == "put" {
            let mut forged = checked.clone();
            let local = forged
                .locals
                .iter_mut()
                .find(|l| l.ty.mut_slice_element().is_some())
                .unwrap();
            let ir::Ty::Ref { kind, .. } = &mut local.ty else {
                unreachable!()
            };
            *kind = ir::BorrowKind::Shared;
            assert!(
                forged
                    .validate_with_facts(ir::FactsCtx::Validated(&facts))
                    .is_err()
            );
            let template = cfg::lower_template(Arc::new(checked), facts.clone(), &budget).unwrap();
            let mut forged_cfg = template.function().as_ref().clone();
            let local = forged_cfg
                .locals
                .iter_mut()
                .find(|l| l.ty.mut_slice_element().is_some())
                .unwrap();
            let ir::Ty::Ref { kind, .. } = &mut local.ty else {
                unreachable!()
            };
            *kind = ir::BorrowKind::Shared;
            assert!(
                forged_cfg
                    .validate_with_facts(ir::FactsCtx::Validated(&facts))
                    .is_err()
            );
        } else {
            let mut forged = checked;
            let expression = forged
                .exprs
                .iter_mut()
                .find(|e| matches!(e.kind, ir::ExprKind::ArrayToSlice { .. }))
                .unwrap();
            let ir::Ty::Ref { kind, .. } = &mut expression.ty else {
                unreachable!()
            };
            *kind = ir::BorrowKind::Shared;
            assert!(
                forged
                    .validate_with_facts(ir::FactsCtx::Validated(&facts))
                    .is_err()
            );
        }
    }
}
