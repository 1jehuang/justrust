//! External-file module acceptance through the native CLI, with bounded children.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
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
            let path = base.join(format!(
                "jr-external-modules-{}-{epoch}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create scratch {}: {error}", path.display()),
            }
        }
        panic!("could not allocate scratch directory");
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn write(&self, name: &str, source: &str) {
        use std::io::Write;
        let path = self.path(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        new_file(&path).write_all(source.as_bytes()).unwrap();
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
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}
impl Captured {
    fn success(&self) {
        assert!(
            self.status.success(),
            "failed: {self:?}\n{}",
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
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(start.elapsed() < timeout, "timed out: {command:?}");
        std::thread::sleep(Duration::from_millis(5));
    };
    Captured {
        status,
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
    }
}
fn command(scratch: &Scratch, workers: usize, check: bool) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    // Resolve modules relative to the input, never the working directory.
    command
        .current_dir(&scratch.0)
        .arg(scratch.path("src/input.rs"))
        .args(["--workers", &workers.to_string()]);
    if check {
        command.arg("--check");
    } else {
        command.arg("-o").arg(scratch.path("program"));
    }
    command
}
fn fixture(files: &[(&str, &str)]) -> Scratch {
    let scratch = Scratch::new();
    for (name, source) in files {
        scratch.write(name, source);
    }
    scratch
}
fn runs(files: &[(&str, &str)], expected: &str) {
    for workers in [1, 8] {
        let scratch = fixture(files);
        capture(
            &mut command(&scratch, workers, true),
            &scratch,
            COMPILE_TIMEOUT,
        )
        .success();
        assert!(
            !scratch.path("program").exists(),
            "check published executable"
        );
        capture(
            &mut command(&scratch, workers, false),
            &scratch,
            COMPILE_TIMEOUT,
        )
        .success();
        let bytes = fs::read(scratch.path("program")).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(
            &mut Command::new(scratch.path("program")),
            &scratch,
            RUN_TIMEOUT,
        );
        result.success();
        assert_eq!(result.stdout, expected.as_bytes(), "workers={workers}");
        assert!(result.stderr.is_empty(), "runtime stderr: {result:?}");
    }
}
fn rejects_fixture(label: &str, scratch: &Scratch) {
    for workers in [1, 8] {
        for check in [false, true] {
            let result = capture(
                &mut command(scratch, workers, check),
                scratch,
                COMPILE_TIMEOUT,
            );
            assert!(
                !result.status.success(),
                "{label}: accepted with workers={workers}, check={check}"
            );
            assert!(
                result.status.code().is_some(),
                "{label}: compiler terminated by signal: {result:?}"
            );
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: compiler panicked: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: missing diagnostic");
            assert!(
                !scratch.path("program").exists(),
                "{label}: published executable"
            );
        }
    }
}
fn rejects(label: &str, files: &[(&str, &str)]) {
    rejects_fixture(label, &fixture(files));
}

#[test]
fn external_flat_and_directory_modules_build_and_check() {
    for module in ["src/foo.rs", "src/foo/mod.rs"] {
        runs(
            &[
                (
                    "src/input.rs",
                    "mod foo; fn main() { if foo::answer() == 42 { println!(\"external ok\"); } }",
                ),
                (module, "pub fn answer() -> i32 { 42 }"),
            ],
            "external ok\n",
        );
    }
}

#[test]
fn external_nested_and_inline_module_directories() {
    for outer in ["src/outer.rs", "src/outer/mod.rs"] {
        runs(
            &[
                (
                    "src/input.rs",
                    "mod outer; mod inline { pub mod leaf; } fn main() { if outer::sum() + inline::leaf::value() == 28 { println!(\"nested ok\"); } }",
                ),
                (
                    outer,
                    "pub mod leaf; pub mod inline { pub mod deeper; } pub fn sum() -> i32 { leaf::value() + inline::deeper::value() }",
                ),
                ("src/outer/leaf.rs", "pub fn value() -> i32 { 7 }"),
                (
                    "src/outer/inline/deeper/mod.rs",
                    "pub fn value() -> i32 { 11 }",
                ),
                ("src/inline/leaf.rs", "pub fn value() -> i32 { 10 }"),
            ],
            "nested ok\n",
        );
    }
}

#[test]
fn external_modules_share_import_resolution_and_ancestor_privacy() {
    runs(
        &[
            (
                "src/input.rs",
                "use crate::outer::child::run as answer; fn secret() -> i32 { 5 } mod outer; fn main() { if answer() == 23 { println!(\"privacy ok\"); } }",
            ),
            (
                "src/outer.rs",
                "fn secret() -> i32 { 7 } mod hidden; pub mod child;",
            ),
            ("src/outer/hidden.rs", "pub fn value() -> i32 { 11 }"),
            (
                "src/outer/child.rs",
                "pub fn run() -> i32 { super::secret() + crate::secret() + super::hidden::value() }",
            ),
        ],
        "privacy ok\n",
    );
}

#[test]
fn external_missing_ambiguous_and_unused_invalid_modules_reject() {
    rejects(
        "missing used module",
        &[("src/input.rs", "mod foo; fn main() { foo::f(); }")],
    );
    rejects(
        "missing unused module",
        &[("src/input.rs", "mod foo; fn main() {}")],
    );
    rejects(
        "missing unused nested module",
        &[
            ("src/input.rs", "mod outer; fn main() {}"),
            ("src/outer.rs", "mod missing;"),
        ],
    );
    rejects(
        "ambiguous module",
        &[
            ("src/input.rs", "mod foo; fn main() {}"),
            ("src/foo.rs", ""),
            ("src/foo/mod.rs", ""),
        ],
    );
    rejects(
        "unused invalid body",
        &[
            ("src/input.rs", "mod foo; fn main() {}"),
            ("src/foo.rs", "fn unused() { missing(); }"),
        ],
    );
    rejects(
        "external parse error",
        &[
            ("src/input.rs", "mod foo; fn main() {}"),
            ("src/foo.rs", "pub fn broken( {"),
        ],
    );
}

#[test]
fn external_private_functions_and_modules_reject() {
    rejects(
        "parent cannot call private child function",
        &[
            ("src/input.rs", "mod foo; fn main() { foo::secret(); }"),
            ("src/foo.rs", "fn secret() {}"),
        ],
    );
    rejects(
        "sibling cannot call private function",
        &[
            ("src/input.rs", "mod foo; mod sibling; fn main() {}"),
            ("src/foo.rs", "fn secret() {}"),
            (
                "src/sibling.rs",
                "pub fn unused() { crate::foo::secret(); }",
            ),
        ],
    );
    rejects(
        "private nested external module",
        &[
            ("src/input.rs", "mod foo; fn main() { foo::hidden::f(); }"),
            ("src/foo.rs", "mod hidden;"),
            ("src/foo/hidden.rs", "pub fn f() {}"),
        ],
    );
}

#[test]
fn external_symlink_cycles_are_bounded_and_rejected() {
    use std::os::unix::fs::symlink;
    let scratch = fixture(&[("src/input.rs", "mod cycle; fn main() {}")]);
    symlink(scratch.path("src/input.rs"), scratch.path("src/cycle.rs")).unwrap();
    rejects_fixture("source file cycle", &scratch);
}

#[test]
fn external_excessive_nesting_is_bounded_and_rejected() {
    let scratch = fixture(&[("src/input.rs", "mod n; fn main() {}")]);
    let mut directory = PathBuf::from("src");
    for _ in 0..160 {
        directory.push("n");
        scratch.write(directory.join("mod.rs").to_str().unwrap(), "pub mod n;");
    }
    scratch.write(directory.join("n.rs").to_str().unwrap(), "pub fn leaf() {}");
    rejects_fixture("external nesting bound", &scratch);
}

#[test]
fn external_source_size_is_bounded_and_rejected() {
    let scratch = fixture(&[("src/input.rs", "mod large; fn main() {}")]);
    // A valid comment isolates the loader's source budget from parser limits.
    let source = format!("/*{}*/", "x".repeat(8 * 1024 * 1024));
    scratch.write("src/large.rs", &source);
    rejects_fixture("external source byte bound", &scratch);
}

#[test]
fn external_path_attributes_are_an_explicit_unsupported_boundary() {
    // Stock Rust supports this. The native milestone must reject it rather
    // than silently loading the default file or ignoring the attribute.
    rejects(
        "unsupported path attribute",
        &[
            (
                "src/input.rs",
                "#[path = \"elsewhere.rs\"] mod foo; fn main() {}",
            ),
            ("src/elsewhere.rs", "pub fn value() {}"),
            ("src/foo.rs", "pub fn value() {}"),
        ],
    );
}

#[test]
fn external_loading_respects_comments_strings_and_file_boundaries() {
    runs(
        &[
            (
                "src/input.rs",
                "// mod missing;\n/* mod other; */ mod foo; fn main() { foo::run(); println!(\"mod phantom;\"); }",
            ),
            (
                "src/foo.rs",
                "pub fn run() {} // trailing comment without newline",
            ),
        ],
        "mod phantom;\n",
    );
    rejects(
        "child cannot close parent module",
        &[
            ("src/input.rs", "mod foo; fn main() {}"),
            ("src/foo.rs", "} pub fn escaped() {} mod replacement {"),
        ],
    );
}

#[test]
fn external_source_graph_is_shared_and_accounts_for_all_files() {
    const CHILD: &str = "JR_EXTERNAL_SOURCE_GRAPH_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use justrust_engine::{Key, compiler};
        let files = [
            ("src/input.rs", "mod foo; fn main() { foo::value(); }"),
            ("src/foo.rs", "pub fn value() -> i32 { 7 }"),
        ];
        let scratch = fixture(&files);
        for workers in [1, 8] {
            for check in [false, true] {
                let result =
                    compiler::compile_file_observed(scratch.path("src/input.rs"), workers, check)
                        .unwrap();
                assert_eq!(
                    result.source_bytes,
                    files.iter().map(|(_, text)| text.len()).sum::<usize>()
                );
                assert_eq!(result.source_files, files.len());
                assert_eq!(result.executable.is_none(), check);
                assert!(result.report.capture_complete);
                assert_eq!(result.report.events_dropped, 0);
                assert_eq!(result.report.failures, 0);
                let source = Key::new(107, vec![]);
                let entries: Vec<_> = result
                    .report
                    .node_evaluations
                    .iter()
                    .filter(|(key, _)| *key == source)
                    .collect();
                assert_eq!(entries.len(), 1, "source operation must be shared");
                assert_eq!(entries[0].1, 1, "source operation repeated");
                let error =
                    compiler::compile_file_observed(scratch.path("src/missing.rs"), workers, check)
                        .err()
                        .expect("missing source must fail");
                let report = error
                    .report
                    .expect("source failure must retain observed report");
                assert!(report.capture_complete);
                assert!(report.failures > 0);
                assert!(
                    report
                        .node_evaluations
                        .iter()
                        .any(|(key, count)| *key == source && *count == 1)
                );
            }
        }
        return;
    }
    let scratch = Scratch::new();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .args([
            "--exact",
            "external_source_graph_is_shared_and_accounts_for_all_files",
            "--nocapture",
        ])
        .env(CHILD, "1");
    capture(&mut child, &scratch, COMPILE_TIMEOUT).success();
}

#[test]
fn external_missing_module_cannot_be_created_by_trace_publication() {
    for workers in [1, 8] {
        for check in [false, true] {
            let scratch = fixture(&[("src/input.rs", "mod foo; fn main() {}")]);
            let trace = scratch.path("src/foo.rs");
            let mut compiler = command(&scratch, workers, check);
            compiler.arg("--trace").arg(&trace);
            let result = capture(&mut compiler, &scratch, COMPILE_TIMEOUT);
            assert!(!result.status.success(), "trace satisfied missing module");
            assert!(
                result.status.code().is_some(),
                "compiler crashed: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "missing source diagnostic");
            assert!(!scratch.path("program").exists(), "published executable");
            let report = fs::read_to_string(trace).expect("source failure must publish trace");
            assert!(
                report.contains("compilation failed"),
                "missing failure trace: {report}"
            );
        }
    }
}
