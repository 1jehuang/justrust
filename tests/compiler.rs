//! End-to-end acceptance tests for the native compiler, not the graph fixtures.
//! No guest rustc, shell, linker, or third-party test dependency is required.
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
            let path = base.join(format!(
                "jr-rust-acceptance-{}-{epoch}-{id}",
                std::process::id()
            ));
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

    fn write(&self, name: &str, contents: &str) -> PathBuf {
        let path = self.path(name);
        let mut file = new_file(&path);
        file.write_all(contents.as_bytes()).unwrap();
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

// A guard also reaps the child if polling or an assertion panics.
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
    fn success(&self) {
        assert!(!self.timed_out, "process timed out: {self:?}");
        assert!(
            self.status.success(),
            "process failed: {self:?}\nstderr: {}",
            String::from_utf8_lossy(&self.stderr)
        );
    }
}

fn capture(command: &mut Command, scratch: &Scratch, timeout: Duration) -> Captured {
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let stdout = scratch.path(&format!("stdout-{id}"));
    let stderr = scratch.path(&format!("stderr-{id}"));
    // Files, rather than undrained pipes, prevent verbose children deadlocking.
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
        stdout: fs::read(stdout).unwrap(),
        stderr: fs::read(stderr).unwrap(),
        timed_out,
    }
}

fn compiler(source: &Path, output: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command.arg(source).arg("-o").arg(output);
    command
}

fn compile(scratch: &Scratch, source: &str, workers: usize) -> PathBuf {
    let input = scratch.write("input.rs", source);
    let output = scratch.path("program");
    let result = capture(
        compiler(&input, &output).args(["--workers", &workers.to_string()]),
        scratch,
        COMPILE_TIMEOUT,
    );
    result.success();
    let bytes = fs::read(&output).expect("successful compile must create executable");
    assert!(bytes.len() >= 64, "truncated native executable");
    assert_eq!(
        &bytes[..4],
        b"\x7fELF",
        "must be a real ELF, not a shell script"
    );
    assert_eq!(bytes[4], 2, "ELF64 required");
    assert_eq!(bytes[5], 1, "little-endian required");
    assert_eq!(
        u16::from_le_bytes([bytes[18], bytes[19]]),
        62,
        "x86_64 required"
    );
    output
}

fn runs(source: &str, stdout: &str, workers: usize) {
    let scratch = Scratch::new();
    let output = compile(&scratch, source, workers);
    let result = capture(&mut Command::new(output), &scratch, RUN_TIMEOUT);
    result.success();
    assert_eq!(result.stdout, stdout.as_bytes(), "source:\n{source}");
    assert!(
        result.stderr.is_empty(),
        "unexpected runtime stderr: {result:?}"
    );
}

#[test]
fn lightweight_profiles_preserve_native_output_and_report_failures() {
    let scratch = Scratch::new();
    let source = scratch.write("profile.rs", "fn main(){println!(\"profile\");}");
    let ordinary = scratch.path("ordinary");
    capture(&mut compiler(&source, &ordinary), &scratch, COMPILE_TIMEOUT).success();
    for workers in [1, 4] {
        let output = scratch.path(&format!("profiled-{workers}"));
        let profile = scratch.path(&format!("profile-{workers}.json"));
        capture(
            compiler(&source, &output)
                .args(["--workers", &workers.to_string(), "--profile"])
                .arg(&profile),
            &scratch,
            COMPILE_TIMEOUT,
        )
        .success();
        assert_eq!(fs::read(&ordinary).unwrap(), fs::read(&output).unwrap());
        let contents = fs::read_to_string(&profile).unwrap();
        assert!(contents.contains("\"success\":true"), "{contents}");
        assert!(contents.contains("\"first_failure\":null"), "{contents}");
        assert!(contents.contains("\"top_nodes\":["), "{contents}");
        assert!(contents.contains("\"unmatched_intervals\":0"), "{contents}");
        assert!(contents.len() < 100_000, "profile must omit CFG payloads");
        let before = fs::read(&profile).unwrap();
        let collision = capture(
            Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&source)
                .args(["--check", "--profile"])
                .arg(&profile),
            &scratch,
            COMPILE_TIMEOUT,
        );
        assert!(!collision.status.success());
        assert_eq!(before, fs::read(&profile).unwrap());
    }
    let bad_source = scratch.write("invalid-profile.rs", "fn main(){let x:i32=true;}");
    let output = scratch.path("invalid-program");
    let profile = scratch.path("failure-profile.json");
    let failure = capture(
        compiler(&bad_source, &output)
            .arg("--profile")
            .arg(&profile),
        &scratch,
        COMPILE_TIMEOUT,
    );
    assert!(!failure.status.success());
    assert!(!output.exists());
    let contents = fs::read_to_string(profile).unwrap();
    assert!(contents.contains("\"success\":false"), "{contents}");
    assert!(contents.contains("\"planning_budget\":{"), "{contents}");
    assert!(contents.contains("\"graph\":{"), "{contents}");

    let growing = scratch.write(
        "resource-profile.rs",
        "fn grow<T>(x:T,n:i32)->i32{if n==0{7}else{grow::<(T,)>((x,),n-1)}}fn main(){let x=grow::<u8>(1u8,1);}",
    );
    for workers in [1, 4] {
        let profile = scratch.path(&format!("resource-profile-{workers}.json"));
        let failure = capture(
            Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&growing)
                .args(["--check", "--workers", &workers.to_string(), "--profile"])
                .arg(&profile),
            &scratch,
            COMPILE_TIMEOUT,
        );
        assert!(!failure.status.success());
        let contents = fs::read_to_string(profile).unwrap();
        assert!(contents.contains("\"success\":false"), "{contents}");
        assert!(contents.contains("\"kind\":\"depth\""), "{contents}");
        assert!(contents.contains("\"requested\":129"), "{contents}");
        assert!(contents.contains("\"limit\":128"), "{contents}");
        assert!(
            contents.contains("\"first_failure_task\":{\"node\":"),
            "{contents}"
        );
        assert!(contents.contains("\"phase\":\"mono_plan\""), "{contents}");
    }
}

#[test]
fn representative_examples_are_real_executables_with_multiple_worker_counts() {
    for workers in [1, 2, 4] {
        runs(
            include_str!("../examples/compiler/hello.rs"),
            "Hello from jr-rust!\n",
            workers,
        );
        runs(
            include_str!("../examples/compiler/arithmetic.rs"),
            "arithmetic ok\n",
            workers,
        );
    }
    runs(
        include_str!("../examples/compiler/hello.rs"),
        "Hello from jr-rust!\n",
        96,
    );
}

#[test]
fn recursive_factorial_and_forward_calls() {
    let source = r#"
fn main() {
    if factorial(6) == 720 { println!("factorial ok"); } else { println!("wrong"); }
}
fn factorial(n: i32) -> i32 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}
"#;
    for workers in [1, 4] {
        runs(source, "factorial ok\n", workers);
    }
}

#[test]
fn while_assignment_and_early_return() {
    runs(
        r#"
fn sum(limit: i32) -> i32 {
    let mut n: i32 = 0;
    let mut total = 0;
    while n < limit { n = n + 1; total = total + n; }
    return total;
}
fn main() {
    if sum(10) == 55 { println!("while ok"); }
    return;
    println!("unreachable");
}
"#,
        "while ok\n",
        3,
    );
}

#[test]
fn signed_division_remainder_precedence_and_integer_boundaries() {
    runs(
        r#"
fn main() {
    let min: i32 = -2147483648;
    let max = 2147483647;
    if -7 / 3 == -2 && -7 % 3 == -1 && 7 / -3 == -2 && 7 % -3 == 1 {
        if -7 / -3 == 2 && -7 % -3 == -1 && 2 + 3 * 4 == 14 {
            if (2 + 3) * 4 == 20 && min + max == -1 && min / 1 == min {
                println!("math ok");
            }
        }
    }
}
"#,
        "math ok\n",
        2,
    );
}

#[test]
fn lexical_scopes_shadowing_and_if_block_values() {
    runs(
        r#"
fn main() {
    let x = 10;
    let y: i32 = { let x = x + 1; x * 2 };
    { let x = true; if x { println!("inner"); } }
    let x = x + 2;
    let z = if x == 12 { y } else { 0 };
    if z == 22 { println!("scope ok"); }
}
"#,
        "inner\nscope ok\n",
        4,
    );
}

#[test]
fn bool_and_unit_arguments_returns_and_operators() {
    runs(
        r#"
fn unit(value: ()) -> () { value }
fn negate(value: bool) -> bool { !value }
fn choose(flag: bool, a: i32, b: i32) -> i32 { if flag { a } else { b } }
fn main() {
    let u: () = unit(());
    unit(u);
    let flag: bool = negate(false);
    if flag && true || false {
        if choose(flag, 8, 9) == 8 && true != false && false == false {
            if 1 < 2 && 2 <= 2 && 3 > 2 && 3 >= 3 { println!("types ok"); }
        }
    }
}
"#,
        "types ok\n",
        2,
    );
}

#[test]
fn logical_operators_short_circuit_and_preserve_precedence() {
    runs(
        r#"
fn noisy() -> bool { println!("must not run"); true }
fn main() {
    let a = false && noisy();
    let b = true || noisy();
    if !a && b && (true || false && false) { println!("short circuit ok"); }
}
"#,
        "short circuit ok\n",
        4,
    );
}

#[test]
fn function_arguments_and_binary_operands_evaluate_left_to_right() {
    runs(
        r#"
fn first() -> i32 { println!("first"); 10 }
fn second() -> i32 { println!("second"); 3 }
fn subtract(a: i32, b: i32) -> i32 { a - b }
fn main() {
    if subtract(first(), second()) == 7 { println!("call ok"); }
    if first() - second() == 7 { println!("binary ok"); }
}
"#,
        "first\nsecond\ncall ok\nfirst\nsecond\nbinary ok\n",
        4,
    );
}

#[test]
fn println_decodes_escapes_and_unicode_exactly() {
    runs(
        r#"fn main() {
    println!("line\nnext\t\"quoted\"\\end\r");
    println!("héllø 世界 🦀");
    println!("\u{1f980}\x21\0");
    println!("");
}"#,
        "line\nnext\t\"quoted\"\\end\r\nhéllø 世界 🦀\n🦀!\0\n\n",
        1,
    );
}

#[test]
fn mutual_recursion_and_nested_returns() {
    runs(
        r#"
fn even(n: i32) -> bool {
    if n == 0 { return true; }
    odd(n - 1)
}
fn odd(n: i32) -> bool {
    if n == 0 { return false; }
    even(n - 1)
}
fn main() {
    if even(20) && odd(19) && !odd(20) { println!("recursion ok"); }
}
"#,
        "recursion ok\n",
        4,
    );
}

#[test]
fn nested_assignment_updates_the_correct_binding() {
    runs(
        r#"
fn main() {
    let mut x = 1;
    { x = x + 1; }
    {
        let mut x = 100;
        { x = x + 5; }
        if x == 105 { println!("inner ok"); }
    }
    if x == 2 { println!("outer ok"); }
}
"#,
        "inner ok\nouter ok\n",
        2,
    );
}

#[test]
fn checked_runtime_arithmetic_traps_exit_101() {
    // Parameters keep these genuine runtime traps in stock Rust as well,
    // without requiring an arithmetic_overflow/unconditional_panic lint override.
    for (expression, x, y) in [
        ("x + y", i32::MAX, 1),
        ("x - y", i32::MIN, 1),
        ("x * y", i32::MAX, 2),
        ("x / y", 7, 0),
        ("x % y", 7, 0),
        ("x / y", i32::MIN, -1),
        ("x % y", i32::MIN, -1),
        ("-x", i32::MIN, 0),
    ] {
        let source = format!(
            "fn trap(x: i32, y: i32) {{ let z = {expression}; println!(\"unreachable\"); }} fn main() {{ trap({x}, {y}); }}"
        );
        let scratch = Scratch::new();
        let output = compile(&scratch, &source, 2);
        let result = capture(&mut Command::new(output), &scratch, RUN_TIMEOUT);
        assert!(!result.timed_out, "trap hung for {expression}: {result:?}");
        assert_eq!(result.status.code(), Some(101), "{expression}: {result:?}");
        assert!(
            result.stdout.is_empty(),
            "continued after trap: {expression}"
        );
    }
}

#[test]
fn invalid_programs_fail_without_publishing_an_executable() {
    let cases = [
        ("unknown value", "fn main() { let x = missing; }"),
        ("unknown function", "fn main() { absent(); }"),
        ("unknown type", "fn main() { let x: Mystery = 1; }"),
        ("immutable assignment", "fn main() { let x = 1; x = 2; }"),
        (
            "immutable parameter",
            "fn f(x: i32) { x = 2; } fn main() {}",
        ),
        ("duplicate functions", "fn f() {} fn f() {} fn main() {}"),
        ("missing argument", "fn f(x: i32) {} fn main() { f(); }"),
        ("extra argument", "fn f() {} fn main() { f(1); }"),
        (
            "wrong argument type",
            "fn f(x: i32) {} fn main() { f(true); }",
        ),
        ("wrong return type", "fn f() -> bool { 1 } fn main() {}"),
        ("missing return", "fn f() -> i32 {} fn main() {}"),
        ("wrong let type", "fn main() { let x: bool = 1; }"),
        (
            "wrong assignment type",
            "fn main() { let mut x = 1; x = false; }",
        ),
        ("wrong condition", "fn main() { if 1 {} }"),
        ("wrong while condition", "fn main() { while 1 {} }"),
        (
            "branch type mismatch",
            "fn main() { let x = if true { 1 } else { false }; }",
        ),
        ("scope escape", "fn main() { { let x = 1; } let y = x; }"),
        ("integer overflow", "fn main() { let x = 2147483648; }"),
        ("missing main", "fn f() {}"),
        (
            "invalid unused body",
            "fn unused() { missing(); } fn main() {}",
        ),
        ("main arguments", "fn main(x: i32) {}"),
        ("main return", "fn main() -> i32 { 0 }"),
        ("formatted println", "fn main() { println!(\"{}\", 1); }"),
        ("nonliteral println", "fn main() { println!(1); }"),
        ("noniterable for loop", "fn main() { for x in 2 {} }"),
        ("struct", "struct X {} fn main() {}"),
        ("malformed syntax", "fn main( {"),
    ];
    for (label, source) in cases {
        let scratch = Scratch::new();
        let input = scratch.write("input.rs", source);
        let output = scratch.path("program");
        let result = capture(&mut compiler(&input, &output), &scratch, COMPILE_TIMEOUT);
        assert!(!result.timed_out, "{label}: compiler hung");
        assert!(!result.status.success(), "{label}: accepted invalid source");
        assert!(!result.stderr.is_empty(), "{label}: missing diagnostic");
        assert!(!output.exists(), "{label}: published invalid executable");
    }
}

#[test]
fn historical_u32_declaration_now_compiles_and_executes() {
    let source = "fn main() { let x: u32 = 1; }";
    for workers in [1, 8] {
        runs(source, "", workers);
    }
}

#[test]
fn historical_array_fixture_now_compiles_and_executes() {
    let scratch = Scratch::new();
    let input = scratch.write("input.rs", "fn main() { let x = [1, 2]; }");
    let output = scratch.path("program");
    capture(&mut compiler(&input, &output), &scratch, COMPILE_TIMEOUT).success();
    capture(&mut Command::new(&output), &scratch, RUN_TIMEOUT).success();
}

#[test]
fn check_validates_without_emitting_or_running() {
    let scratch = Scratch::new();
    let input = scratch.write("input.rs", "fn main() { while true {} }");
    let output = scratch.path("program");
    let mut check = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    check.arg(&input).args(["--check", "--workers", "2"]);
    let result = capture(&mut check, &scratch, COMPILE_TIMEOUT);
    result.success();
    assert!(!output.exists(), "--check must not emit an executable");
    let invalid = scratch.write("invalid.rs", "fn main() { absent(); }");
    let mut check = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    check.arg(&invalid).arg("--check");
    let result = capture(&mut check, &scratch, COMPILE_TIMEOUT);
    assert!(!result.timed_out);
    assert!(
        !result.status.success(),
        "--check must perform semantic checks"
    );
    assert!(!output.exists());
}

#[test]
fn existing_outputs_are_never_overwritten() {
    let scratch = Scratch::new();
    let input = scratch.write("input.rs", "fn main() {}");
    let output = scratch.write("program", "sentinel: preserve this file\n");
    let result = capture(&mut compiler(&input, &output), &scratch, COMPILE_TIMEOUT);
    assert!(!result.timed_out);
    assert!(!result.status.success(), "must reject existing -o path");
    assert_eq!(
        fs::read(&output).unwrap(),
        b"sentinel: preserve this file\n"
    );
    // A dangling symlink is also an existing output, not a vacant filename.
    let link = scratch.path("link");
    let target = scratch.path("must-not-create");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let result = capture(&mut compiler(&input, &link), &scratch, COMPILE_TIMEOUT);
    assert!(!result.timed_out);
    assert!(!result.status.success());
    assert!(
        fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink()
    );
    assert!(!target.exists());
}

#[test]
fn trace_is_created_but_existing_trace_is_preserved() {
    let scratch = Scratch::new();
    let input = scratch.write("input.rs", "fn main() { println!(\"trace ok\"); }");
    let output = scratch.path("program");
    let trace = scratch.path("trace.json");
    let result = capture(
        compiler(&input, &output).arg("--trace").arg(&trace),
        &scratch,
        COMPILE_TIMEOUT,
    );
    result.success();
    let before = fs::read(&trace).expect("trace must be created");
    assert!(!before.is_empty(), "trace must contain accounting");
    let other = scratch.path("other-program");
    let result = capture(
        compiler(&input, &other).arg("--trace").arg(&trace),
        &scratch,
        COMPILE_TIMEOUT,
    );
    assert!(!result.timed_out);
    assert!(!result.status.success(), "must reject existing trace");
    assert_eq!(fs::read(&trace).unwrap(), before);
}

#[test]
fn cli_rejects_invalid_workers_missing_values_and_unknown_flags() {
    for args in [
        vec!["--workers", "0"],
        vec!["--workers", "many"],
        vec!["--workers"],
        vec!["--trace"],
        vec!["--unknown"],
    ] {
        let scratch = Scratch::new();
        let input = scratch.write("input.rs", "fn main() {}");
        let output = scratch.path("program");
        let result = capture(
            compiler(&input, &output).args(&args),
            &scratch,
            COMPILE_TIMEOUT,
        );
        assert!(!result.timed_out, "{args:?}");
        assert!(!result.status.success(), "accepted {args:?}");
        assert!(!output.exists());
    }
}

#[test]
fn invalid_unused_body_retains_failure_trace_without_publishing_executable() {
    let scratch = Scratch::new();
    let source = "fn unused() { missing(); } fn main() {}";
    let input = scratch.write("input.rs", source);
    let output = scratch.path("program");
    let trace = scratch.path("failed.trace");
    let result = capture(
        compiler(&input, &output).arg("--trace").arg(&trace),
        &scratch,
        COMPILE_TIMEOUT,
    );
    assert!(!result.timed_out);
    assert!(!result.status.success());
    assert!(!result.stderr.is_empty());
    assert!(
        !output.exists(),
        "unused invalid body must gate publication"
    );
    let text = fs::read_to_string(trace).expect("failure trace must still be published");
    assert!(text.contains("node_errors:"), "{text}");
    assert!(
        !text.contains("node_errors: []"),
        "must retain node failures: {text}"
    );
    assert!(text.contains("dependency_edges:"), "{text}");
    assert!(
        !text.contains("dependency_edges: []"),
        "must retain dependency edges: {text}"
    );
}

#[test]
fn infinite_generated_program_is_killed_and_reaped_by_timeout() {
    let scratch = Scratch::new();
    let output = compile(&scratch, "fn main() { while true {} }", 2);
    let result = capture(
        &mut Command::new(output),
        &scratch,
        Duration::from_millis(250),
    );
    assert!(
        result.timed_out,
        "infinite program unexpectedly terminated: {result:?}"
    );
    assert!(!result.status.success());
}

#[test]
fn compiler_graph_shares_check_and_build_and_check_only_never_demands_codegen() {
    const CHILD: &str = "JR_RUST_ACCEPTANCE_GRAPH_CHILD";
    if std::env::var_os(CHILD).is_some() {
        use justrust_engine::{Key, compiler};

        let source = "fn double(n: i32) -> i32 { n * 2 } fn unused() {} fn main() { double(3); }";
        for workers in [1, 8] {
            for check_only in [true, false] {
                let result = compiler::compile(source.to_owned(), workers, check_only).unwrap();
                assert_eq!(result.functions, 3);
                assert_eq!(result.analyses.len(), result.functions);
                assert!(result.analyses.iter().all(|analysis| analysis.passed()));
                assert_eq!(result.executable.is_none(), check_only);
                let report = result.report;
                assert!(report.capture_complete);
                assert_eq!(report.failures, 0);
                assert_eq!(report.events_dropped, 0);
                // These operation IDs are the compiler graph schema, not fixtures.
                let mut expected = vec![
                    Key::new(107, vec![0]), // Root source file unit.
                    Key::new(100, vec![]),
                    Key::new(106, vec![]),
                    Key::new(101, vec![]),
                    Key::new(104, vec![]),
                    Key::new(113, vec![]),
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
                    assert!(report.intern_hits > 0, "check and build should share work");
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
                            if plan.callable_env(declaration).unwrap().is_generic() {
                                for kind in [111, 112, 115] {
                                    expected.push(Key::new(kind, vec![index as u64]));
                                }
                            }
                        }
                        compiler::frontend::CallableProvider::Recipe { .. } => {
                            assert!(index >= result.functions);
                            for kind in [110, 111, 112, 115] {
                                expected.push(Key::new(kind, vec![index as u64]));
                            }
                        }
                        compiler::frontend::CallableProvider::UnavailableProvided { .. } => {
                            panic!("unavailable provider entered body enumeration")
                        }
                    }
                }
                // Embedded native library files are independent SOURCE facts.
                expected.extend(
                    [vec![1], vec![1, 0], vec![1, 0, 0], vec![1, 0, 1]]
                        .map(|ordinal| Key::new(107, ordinal)),
                );
                assert_eq!(report.node_evaluations.len(), expected.len());
                for key in expected {
                    let entries: Vec<_> = report
                        .node_evaluations
                        .iter()
                        .filter(|(seen, _)| *seen == key)
                        .collect();
                    assert_eq!(entries.len(), 1, "missing or duplicated node {key:?}");
                    assert_eq!(entries[0].1, 1, "shared node evaluated repeatedly: {key:?}");
                }
            }
        }
        let failure = compiler::compile_observed(
            "fn unused() { missing(); } fn main() {}".to_owned(),
            4,
            false,
        )
        .err()
        .expect("invalid unused body must fail compilation");
        let report = failure
            .report
            .expect("semantic failure must retain its report");
        assert!(!failure.error.to_string().is_empty());
        assert!(report.failures > 0);
        assert!(!report.node_errors.is_empty());
        assert!(!report.dependency_edges.is_empty());
        assert!(!report.events.is_empty());
        assert!(report.capture_complete);
        return;
    }
    // Run the library assertions in a bounded child so an engine deadlock
    // cannot strand the integration suite or the hosted test runner.
    let scratch = Scratch::new();
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.args([
        "--exact",
        "compiler_graph_shares_check_and_build_and_check_only_never_demands_codegen",
        "--nocapture",
    ]);
    child.env(CHILD, "1");
    capture(&mut child, &scratch, COMPILE_TIMEOUT).success();
}
