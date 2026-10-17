//! Native inline-module acceptance tests. Stock Rust is deliberately not used here.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);
const COMPILE_TIMEOUT: Duration = Duration::from_secs(20);
const RUN_TIMEOUT: Duration = Duration::from_secs(5);

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
            let path = base.join(format!("jr-modules-{}-{epoch}-{id}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch {}: {error}", path.display()),
            }
        }
        panic!("could not allocate unique scratch directory");
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.path(name);
        new_file(&path).write_all(text.as_bytes()).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
fn new_file(path: &Path) -> File {
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
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}
impl Captured {
    fn success(&self, source: &str) {
        assert!(!self.timed_out, "timed out: {self:?}\n{source}");
        assert!(
            self.status.success(),
            "failed: {self:?}\n{}\n{source}",
            String::from_utf8_lossy(&self.stderr)
        );
    }
}
fn capture(command: &mut Command, scratch: &Scratch, timeout: Duration) -> Captured {
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
    let (status, timed_out) = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break (status, false);
        }
        if start.elapsed() >= timeout {
            child.0.kill().unwrap();
            break (child.0.wait().unwrap(), true);
        }
        std::thread::sleep(Duration::from_millis(5));
    };
    Captured {
        status,
        timed_out,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn compiler(input: &Path, output: &Path, workers: usize) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command
        .arg(input)
        .arg("-o")
        .arg(output)
        .args(["--workers", &workers.to_string()]);
    command
}
fn runs(source: &str, expected: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.write("input.rs", source);
        let output = scratch.path("program");
        let result = capture(
            &mut compiler(&input, &output, workers),
            &scratch,
            COMPILE_TIMEOUT,
        );
        result.success(source);
        let bytes = fs::read(&output).expect("successful compile must publish executable");
        assert!(bytes.len() >= 64, "truncated native executable");
        assert_eq!(
            &bytes[..6],
            b"\x7fELF\x02\x01",
            "must emit native little-endian ELF64"
        );
        let result = capture(&mut Command::new(output), &scratch, RUN_TIMEOUT);
        result.success(source);
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        assert!(
            result.stderr.is_empty(),
            "unexpected runtime stderr: {result:?}"
        );
    }
}
fn rejects(label: &str, source: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.write("input.rs", source);
        let output = scratch.path("must-not-exist");
        for check in [false, true] {
            let mut command = if check {
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .arg(&input)
                    .args(["--workers", &workers.to_string(), "--check"]);
                command
            } else {
                compiler(&input, &output, workers)
            };
            let result = capture(&mut command, &scratch, COMPILE_TIMEOUT);
            assert!(
                !result.timed_out,
                "{label}: workers={workers}, check={check} hung"
            );
            assert!(
                !result.status.success(),
                "{label}: accepted invalid/unsupported source\n{source}"
            );
            assert!(!result.stderr.is_empty(), "{label}: missing diagnostic");
            assert!(!output.exists(), "{label}: published executable on failure");
        }
    }
}

#[test]
fn nested_inline_modules_paths_and_distinct_canonical_names() {
    runs(
        r#"
fn value() -> i32 { 1 }
mod left {
    pub fn value() -> i32 { 2 }
    pub mod nested {
        pub fn value() -> i32 { 3 }
        pub fn sum() -> i32 { self::value() + super::value() + crate::value() }
    }
    pub fn sum() -> i32 { nested::sum() }
}
pub mod right { pub fn value() -> i32 { 4 } }
fn main() {
    if left::sum() == 6 && right::value() == 4 && left::value() == 2 {
        println!("paths ok");
    }
}
"#,
        "paths ok\n",
    );
}

#[test]
fn cross_module_recursion_and_forward_declarations() {
    runs(
        r#"
fn main() {
    if even(12) && !even(11) && odd(11) { println!("recursion ok"); }
}
use crate::parity::even as even;
use crate::other::odd;
mod parity {
    use crate::other::odd as next;
    pub fn even(n: i32) -> bool { if n == 0 { true } else { next(n - 1) } }
}
mod other {
    pub fn odd(n: i32) -> bool {
        if n == 0 { false } else { super::parity::even(n - 1) }
    }
}
"#,
        "recursion ok\n",
    );
}

#[test]
fn module_and_function_import_aliases_and_relative_uses() {
    runs(
        r#"
use crate::outer::leaf as renamed;
use crate::outer::leaf::value as answer;
mod outer {
    use self::leaf::value as local;
    pub mod leaf { pub fn value() -> i32 { 17 } }
    pub mod child {
        use super::leaf as sibling;
        use super::leaf::value as imported;
        pub fn run() -> i32 { sibling::value() + imported() + super::local() }
    }
    use leaf::value as relative;
    pub fn value() -> i32 { local() + relative() }
}
fn main() {
    if renamed::value() == answer() && outer::child::run() == 51 && outer::value() == 34 {
        println!("imports ok");
    }
}
"#,
        "imports ok\n",
    );
}

#[test]
fn private_ancestor_items_are_accessible_to_descendants() {
    runs(
        r#"
fn secret() -> i32 { 5 }
mod parent {
    fn secret() -> i32 { 7 }
    mod hidden { pub fn value() -> i32 { 11 } }
    pub mod child {
        pub mod grandchild {
            pub fn run() -> i32 { super::super::secret() + crate::secret() + crate::parent::hidden::value() }
        }
    }
}
mod sibling { pub fn run() -> i32 { crate::secret() } }
fn main() {
    if parent::child::grandchild::run() == 23 && sibling::run() == 5 { println!("privacy ok"); }
}
"#,
        "privacy ok\n",
    );
}

#[test]
fn qualified_calls_bypass_local_value_shadowing_and_nested_main_is_not_entry() {
    runs(
        r#"
mod target { pub fn value() -> i32 { 19 } pub fn main() { println!("wrong entry"); } }
fn value() -> i32 { 23 }
fn main() {
    let value = 1;
    let target = 2;
    if self::value() == 23 && target::value() == 19 && value + target == 3 {
        println!("root entry ok");
    }
}
"#,
        "root entry ok\n",
    );
}

#[test]
fn same_spelling_module_and_function_use_separate_namespaces() {
    runs(
        r#"
mod same { pub fn value() -> i32 { 3 } }
fn same() -> i32 { 7 }
fn main() { if same() + same::value() == 10 { println!("namespaces ok"); } }
"#,
        "namespaces ok\n",
    );
}

#[test]
fn forward_import_chains_resolve_without_declaration_order_dependence() {
    runs(
        r#"
use self::alias as final_name;
use self::source::value as alias;
use self::module_alias as final_module;
use self::source as module_alias;
mod source { pub fn value() -> i32 { 29 } }
fn main() {
    if final_name() == 29 && final_module::value() == 29 { println!("forward imports ok"); }
}
"#,
        "forward imports ok\n",
    );
}

#[test]
fn keyword_module_imports_and_disjoint_import_namespaces() {
    runs(
        r#"
use crate as root;
mod a { pub fn f() -> i32 { 13 } }
use a::f as a;
mod b {
    use self as me;
    fn value() -> i32 { 17 }
    pub fn run() -> i32 { me::value() }
}
fn value() -> i32 { 19 }
fn main() {
    if a() + a::f() + b::run() + root::value() == 62 { println!("keyword imports ok"); }
}
"#,
        "keyword imports ok\n",
    );
}

#[test]
fn invalid_module_programs_fail_in_build_and_check_without_output() {
    for (label, source) in [
        ("duplicate module", "mod a {} mod a {} fn main() {}"),
        (
            "duplicate nested function",
            "mod a { fn f() {} fn f() {} } fn main() {}",
        ),
        (
            "duplicate function imports",
            "mod a { pub fn f() {} } use a::f; use a::f; fn main() {}",
        ),
        (
            "function import collision",
            "mod a { pub fn f() {} } use a::f; fn f() {} fn main() {}",
        ),
        (
            "module import collision",
            "mod a {} mod b {} use a as b; fn main() {}",
        ),
        (
            "sibling private function",
            "mod a { fn f() {} } mod b { pub fn g() { crate::a::f(); } } fn main() {}",
        ),
        (
            "parent private child function",
            "mod a { fn f() {} } fn main() { a::f(); }",
        ),
        (
            "private nested module",
            "mod a { mod hidden { pub fn f() {} } } fn main() { a::hidden::f(); }",
        ),
        (
            "private import target",
            "mod a { fn f() {} } use a::f as alias; fn main() {}",
        ),
        (
            "private alias from sibling",
            "mod a { pub fn f() {} } mod b { use crate::a::f as alias; } fn main() { b::alias(); }",
        ),
        ("missing unused import", "use crate::missing; fn main() {}"),
        (
            "missing unused nested import",
            "mod a { use self::missing as unused; } fn main() {}",
        ),
        (
            "missing imported child",
            "mod a {} use a::missing as unused; fn main() {}",
        ),
        (
            "import cycle",
            "use self::b as a; use self::a as b; fn main() {}",
        ),
        ("root super", "fn main() { super::f(); } fn f() {}"),
        (
            "too many super",
            "mod a { pub fn f() { super::super::g(); } } fn g() {} fn main() {}",
        ),
        (
            "missing qualified function",
            "mod a {} fn main() { a::missing(); }",
        ),
        ("function used as module", "fn a() {} fn main() { a::f(); }"),
        ("module called as function", "mod a {} fn main() { a(); }"),
        (
            "local shadows function",
            "fn f() {} fn main() { let f = 1; f(); }",
        ),
        (
            "local shadows import",
            "mod a { pub fn f() {} } use a::f; fn main() { let f = 1; f(); }",
        ),
        ("nested main is not entry", "mod a { pub fn main() {} }"),
        (
            "invalid unused nested body",
            "mod a { fn unused() { missing(); } } fn main() {}",
        ),
        (
            "wrong nested argument",
            "mod a { pub fn f(x: i32) {} } fn main() { a::f(true); }",
        ),
        ("unterminated module", "mod a { fn f() {} fn main() {}"),
        ("missing module name", "mod {} fn main() {}"),
        ("missing use target", "use ; fn main() {}"),
        ("missing alias name", "mod a {} use a as ; fn main() {}"),
        (
            "private function beside public module",
            "mod a { pub mod same {} fn same() {} } use a::same as alias; fn main() { alias(); }",
        ),
        (
            "private module beside public function",
            "mod a { mod same { pub fn f() {} } pub fn same() {} } use a::same as alias; fn main() { alias::f(); }",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn imports_filter_private_candidates_in_each_namespace() {
    runs(
        r#"
mod a { pub mod same { pub fn f() -> i32 { 3 } } fn same() -> i32 { 99 } }
use a::same as alias;
fn main() { if alias::f() == 3 { println!("public module"); } }
"#,
        "public module\n",
    );
    runs(
        r#"
mod a { mod same {} pub fn same() -> i32 { 7 } }
use a::same as alias;
fn main() { if alias() == 7 { println!("public function"); } }
"#,
        "public function\n",
    );
    runs(
        r#"
mod a { mod same {} pub fn same() -> i32 { 11 } }
use a::same;
fn main() { if same() == 11 { println!("unaliased function"); } }
"#,
        "unaliased function\n",
    );
}

#[test]
fn historical_grouped_import_fixture_is_accepted() {
    runs(
        "mod a { pub fn f() {} pub fn g() {} } use a::{f, g}; fn main() {}",
        "",
    );
}

#[test]
fn explicit_module_milestone_boundaries_reject_without_output() {
    // These are intentional feature boundaries, not claims that stock Rust rejects them.
    for (label, source) in [
        (
            "imported main is outside milestone",
            "mod a { pub fn entry() {} } use a::entry as main;",
        ),
        ("external module", "mod external; fn main() {}"),
        (
            "glob import",
            "mod a { pub fn f() {} } use a::*; fn main() {}",
        ),
    ] {
        rejects(label, source);
    }
}

#[test]
fn module_resolution_is_shared_and_check_never_demands_codegen() {
    const CHILD: &str = "JR_MODULE_GRAPH_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use justrust_engine::{Key, compiler};
        let source = "use crate::outer::inner::f as alias; mod outer { pub mod inner { pub fn f() -> i32 { 7 } } fn unused() {} } fn main() { alias(); }";
        for workers in [1, 8] {
            for check_only in [true, false] {
                let result = compiler::compile(source.to_owned(), workers, check_only).unwrap();
                assert_eq!(result.functions, 3);
                assert_eq!(result.analyses.len(), result.functions);
                assert!(result.analyses.iter().all(|analysis| analysis.passed()));
                assert_eq!(result.executable.is_none(), check_only);
                let report = result.report;
                assert!(report.capture_complete);
                assert_eq!(report.events_dropped, 0);
                assert_eq!(report.failures, 0);
                let mut expected = vec![
                    Key::new(100, vec![]),
                    Key::new(106, vec![]),
                    Key::new(101, vec![]),
                    Key::new(104, vec![]),
                    Key::new(113, vec![]),
                    Key::new(115, vec![]),
                ];
                for index in 0..3 {
                    expected.push(Key::new(110, vec![index]));
                    expected.push(Key::new(102, vec![index]));
                    expected.push(Key::new(108, vec![index]));
                    expected.push(Key::new(109, vec![index]));
                    if !check_only {
                        expected.push(Key::new(103, vec![index]));
                    }
                }
                if !check_only {
                    expected.push(Key::new(105, vec![]));
                    expected.push(Key::new(114, vec![]));
                }
                // Available catalog recipes add real universal proof nodes, but
                // never eager concrete roots. Keep every historical source key.
                let program = compiler::frontend::parse(source).unwrap();
                let plan = compiler::frontend::PlanCtx::from_program(
                    &program,
                    &compiler::ir::PlanBudget::default(),
                )
                .unwrap();
                for index in 0..plan.callable_count() {
                    let declaration = plan.callable_decl(index).unwrap();
                    match plan.callable_provider(declaration).unwrap() {
                        compiler::frontend::CallableProvider::Source { .. } => {
                            assert!(index < result.functions);
                        }
                        compiler::frontend::CallableProvider::Recipe { .. } => {
                            assert!(index >= result.functions);
                            for kind in [110, 111, 112] {
                                expected.push(Key::new(kind, vec![index as u64]));
                            }
                        }
                        compiler::frontend::CallableProvider::UnavailableProvided { .. } => {
                            panic!("unavailable provider entered body enumeration")
                        }
                    }
                }
                // Effect publication joins every original body and required universal
                // proof before the instance plan can publish any concrete work.
                let discovered = |key: &Key| {
                    let spelling = format!("{key:?}");
                    report
                        .events
                        .iter()
                        .find(|event| event.kind == "discovered" && event.detail == spelling)
                        .expect("required graph node was not discovered")
                };
                let published = |node| {
                    report
                        .events
                        .iter()
                        .find(|event| event.kind == "publish" && event.node == Some(node))
                        .expect("required graph node did not publish")
                        .at_ns
                };
                let effects = published(discovered(&Key::new(115, vec![])).node.unwrap());
                assert!(published(discovered(&Key::new(113, vec![])).node.unwrap()) >= effects);
                for (key, _) in &report.node_evaluations {
                    let event = discovered(key);
                    if matches!(key.kind, 110 | 112) {
                        assert!(
                            published(event.node.unwrap()) <= effects,
                            "effect publication preceded mandatory body proof: {key:?}"
                        );
                    }
                    if matches!(key.kind, 102 | 108 | 109 | 103) {
                        assert!(
                            event.at_ns >= effects,
                            "closed work preceded effects: {key:?}"
                        );
                    }
                }
                if check_only {
                    assert!(
                        !report
                            .node_evaluations
                            .iter()
                            .any(|(key, _)| key.kind == 103)
                    );
                } else {
                    let sealed = published(discovered(&Key::new(114, vec![])).node.unwrap());
                    for (key, _) in &report.node_evaluations {
                        if key.kind == 103 {
                            assert!(discovered(key).at_ns >= sealed);
                        }
                    }
                }
                assert_eq!(report.node_evaluations.len(), expected.len());
                for key in expected {
                    let entries: Vec<_> = report
                        .node_evaluations
                        .iter()
                        .filter(|(seen, _)| *seen == key)
                        .collect();
                    assert_eq!(entries.len(), 1, "missing or duplicated {key:?}");
                    assert_eq!(entries[0].1, 1, "shared operation repeated {key:?}");
                }
            }
        }
        return;
    }
    let scratch = Scratch::new();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.args([
        "--exact",
        "module_resolution_is_shared_and_check_never_demands_codegen",
        "--nocapture",
    ]);
    child.env(CHILD, "1");
    capture(&mut child, &scratch, COMPILE_TIMEOUT).success("module graph child");
}
