//! Payload enum native acceptance for the bounded tuple/braced/unit milestone.
//! Historical fixtures were stock-classified with rustc 1.98.1, edition 2024.
//! Supported payload transitions and empty enums also run the pinned stock oracle.
//! Guard, or-pattern and Drop controls retain unused-body semantic rejection.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/stock_rustc.rs"]
mod stock;

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
            let path = base.join(format!("jr-payload-{}-{epoch}-{id}", std::process::id()));
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
fn runs(source: &str, expected: &str) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("program");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                result.status.success(),
                "workers={workers}, check={check}: {result:?}\n{}\n{source}",
                String::from_utf8_lossy(&result.stderr)
            );
            if check {
                assert!(!output.exists(), "check published executable");
            }
        }
        let bytes = fs::read(&output).unwrap();
        assert!(bytes.len() >= 64, "truncated ELF");
        assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
        let result = capture(&mut Command::new(output), &scratch, 5);
        assert!(
            result.status.success(),
            "runtime failure: {result:?}\n{source}"
        );
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
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for check in [true, false] {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert!(
                !result.status.success(),
                "{label}: accepted workers={workers}, check={check}\n{source}"
            );
            assert!(
                result.status.code().is_some(),
                "{label}: compiler terminated by signal: {result:?}"
            );
            assert!(!result.stderr.is_empty(), "{label}: no diagnostic");
            assert!(
                !String::from_utf8_lossy(&result.stderr).contains("panicked at"),
                "{label}: compiler panic: {result:?}"
            );
            assert!(!output.exists(), "{label}: published executable");
        }
    }
}

// These historical sources exercise already-supported guarded/or-pattern and
// Drop producers. Require an actual pinned stock oracle and native execution,
// paired with an unused invalid body that must fail semantically in both modes.
fn payload_transition(source: &str, invalid: &str) {
    let rustc = stock::stock_rustc();
    for (text, valid) in [(source, true), (invalid, false)] {
        for check in [true, false] {
            let scratch = Scratch::new();
            let input = scratch.source(text);
            let output = scratch.path(if check { "metadata" } else { "stock" });
            let mut command = Command::new(&rustc);
            command.env_remove("RUSTC_BOOTSTRAP").args([
                "--edition=2024",
                "--crate-name=payload_transition",
                "--color=never",
                "-Cpanic=abort",
            ]);
            if check {
                command.arg("--emit=metadata");
            }
            command.arg(&input).arg("-o").arg(&output);
            let result = capture(&mut command, &scratch, 20);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert_eq!(
                result.status.code(),
                Some(i32::from(!valid)),
                "stock check={check}: {diagnostic}\n{text}"
            );
            if !valid {
                assert!(
                    diagnostic.contains("error[E0308]"),
                    "stock check={check}: {diagnostic}"
                );
                assert!(!output.exists(), "stock published rejected output");
            } else if !check {
                let result = capture(&mut Command::new(output), &scratch, 5);
                assert!(result.status.success(), "stock runtime: {result:?}");
                assert_eq!(result.stdout, b"payload oracle ok\n");
                assert!(result.stderr.is_empty(), "stock runtime: {result:?}");
            }
        }
    }
    runs(source, "payload oracle ok\n");
    for workers in [1, 8] {
        for check in [true, false] {
            let scratch = Scratch::new();
            let input = scratch.source(invalid);
            let output = scratch.path("must-not-exist");
            let mut command = compiler(&input, &output, workers, check);
            command
                .current_dir(&scratch.0)
                .env_remove("RUSTC_BOOTSTRAP")
                .env("RUSTC", scratch.path("no-rustc"))
                .env("PATH", scratch.path("no-tools"));
            let result = capture(&mut command, &scratch, 20);
            let diagnostic = String::from_utf8_lossy(&result.stderr);
            assert_eq!(
                result.status.code(),
                Some(1),
                "workers={workers} check={check}: {diagnostic}"
            );
            assert!(
                diagnostic.contains("E0308") || diagnostic.contains("expected Bool, got Int(I32)"),
                "workers={workers} check={check}: {diagnostic}"
            );
            for boundary in [
                "ResourceLimit",
                "unsupported:",
                "contract:",
                "infrastructure:",
                "panicked at",
            ] {
                assert!(
                    !diagnostic.contains(boundary),
                    "workers={workers} check={check}: {diagnostic}"
                );
            }
            assert!(result.stdout.is_empty());
            assert!(!output.exists(), "published rejected payload executable");
        }
    }
}

// These exact sources are shared by the pinned stock and native gates. No
// inhabited stand-in for Void, Always, OnUnwind or OnSuccess is permitted.
const EMPTY_ENUM_VALID: &[(&str, &str)] = &[
    (
        "scopeguard_strategy_type_use",
        r#"pub trait Strategy{fn should_run()->bool;}
pub enum Always{} pub enum OnUnwind{} pub enum OnSuccess{}
impl Strategy for Always{fn should_run()->bool{true}}
impl Strategy for OnUnwind{fn should_run()->bool{false}}
impl Strategy for OnSuccess{fn should_run()->bool{true}}
fn enabled<S:Strategy>()->bool{S::should_run()}
fn main(){if enabled::<Always>()&&!enabled::<OnUnwind>()&&enabled::<OnSuccess>(){println!("empty enum ok");}}"#,
    ),
    (
        "owned_absurd",
        r#"enum Void{} fn absurd(value:Void)->i32{match value{}}
fn main(){println!("empty enum ok");}"#,
    ),
    (
        "shared_deref_absurd",
        r#"enum Void{} fn absurd(value:&Void)->i32{match *value{}}
fn main(){println!("empty enum ok");}"#,
    ),
    (
        "trait_receiver_absurd",
        r#"enum Void{} trait Absurd{fn absurd(self)->i32;}
impl Absurd for Void{fn absurd(self)->i32{match self{}}}
fn main(){println!("empty enum ok");}"#,
    ),
    (
        "imported_identity",
        r#"mod declaration{pub enum Void{}}use declaration::Void as Empty;
fn absurd(value:Empty)->bool{match value{}}
fn main(){println!("empty enum ok");}"#,
    ),
    (
        "uninhabited_payload_branch",
        r#"enum Void{} enum Choice{Impossible(Void),Value(i32)}
fn value(choice:Choice)->i32{match choice{Choice::Impossible(v)=>match v{},Choice::Value(n)=>n}}
fn main(){if value(Choice::Value(42))==42{println!("empty enum ok");}}"#,
    ),
];

const EMPTY_ENUM_INVALID: &[(&str, &str, &str)] = &[
    (
        "record_constructor",
        "enum Void{}fn unused()->Void{Void{}}fn main(){}",
        "E0574",
    ),
    (
        "unit_constructor",
        "enum Void{}fn unused()->Void{Void}fn main(){}",
        "E0423",
    ),
    (
        "tuple_constructor",
        "enum Void{}fn unused()->Void{Void()}fn main(){}",
        "E0423",
    ),
    (
        "invented_variant",
        "enum Void{}fn unused()->Void{Void::Invented}fn main(){}",
        "E0599",
    ),
    (
        "reference_is_not_empty",
        "enum Void{}fn unused(value:&Void)->i32{match value{}}fn main(){}",
        "E0004",
    ),
    (
        "inhabited_enum_is_not_empty",
        "enum Void{}enum Other{Exists}fn unused(value:Other)->i32{match value{}}fn main(){}",
        "E0004",
    ),
    (
        "empty_arm_body_still_checked",
        "enum Void{}fn unused(value:Void)->i32{match value{_=>true}}fn main(){}",
        "E0308",
    ),
    (
        "unused_type_parameter",
        "enum Void<T>{}fn main(){}",
        "E0392",
    ),
    (
        "unused_lifetime_parameter",
        "enum Void<'a>{}fn main(){}",
        "E0392",
    ),
];

#[test]
fn empty_enum_pinned_stock_check_build_and_runtime_oracle() {
    let rustc = stock::stock_rustc();
    for (name, source, diagnostic) in EMPTY_ENUM_VALID
        .iter()
        .map(|(name, source)| (*name, *source, None))
        .chain(
            EMPTY_ENUM_INVALID
                .iter()
                .map(|(name, source, diagnostic)| (*name, *source, Some(*diagnostic))),
        )
    {
        for check in [true, false] {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            let output = scratch.path(if check { "metadata" } else { "stock" });
            let mut command = Command::new(&rustc);
            command.env_remove("RUSTC_BOOTSTRAP").args([
                "--edition=2024",
                "--crate-name=empty_enum_oracle",
                "--color=never",
                "-Cpanic=abort",
            ]);
            if check {
                command.arg("--emit=metadata");
            }
            command.arg(&input).arg("-o").arg(&output);
            let result = capture(&mut command, &scratch, 20);
            let stderr = String::from_utf8_lossy(&result.stderr);
            if let Some(diagnostic) = diagnostic {
                assert!(!result.status.success(), "{name} check={check}: {source}");
                assert!(
                    stderr.contains(diagnostic),
                    "{name} check={check}: {stderr}"
                );
                assert!(!output.exists(), "{name}: stock published rejected output");
            } else {
                assert!(result.status.success(), "{name} check={check}: {stderr}");
                if !check {
                    let result = capture(&mut Command::new(output), &scratch, 5);
                    assert!(result.status.success(), "{name}: {result:?}");
                    assert_eq!(result.stdout, b"empty enum ok\n", "{name}");
                    assert!(result.stderr.is_empty(), "{name}: {result:?}");
                }
            }
        }
    }
}

#[test]
fn empty_enum_native_check_build_run_workers1_8_no_stock_fallback() {
    for (name, source, valid) in EMPTY_ENUM_VALID
        .iter()
        .map(|(name, source)| (*name, *source, true))
        .chain(
            EMPTY_ENUM_INVALID
                .iter()
                .map(|(name, source, _)| (*name, *source, false)),
        )
    {
        for workers in [1, 8] {
            for check in [true, false] {
                let scratch = Scratch::new();
                let input = scratch.source(source);
                let output = scratch.path("native");
                let mut command = compiler(&input, &output, workers, check);
                command
                    .current_dir(&scratch.0)
                    .env_remove("RUSTC_BOOTSTRAP")
                    .env("RUSTC", scratch.path("no-rustc"))
                    .env("PATH", scratch.path("no-tools"));
                let result = capture(&mut command, &scratch, 20);
                let stderr = String::from_utf8_lossy(&result.stderr);
                assert!(result.status.code().is_some(), "{name}: {result:?}");
                assert!(!stderr.contains("panicked at"), "{name}: {stderr}");
                assert_eq!(
                    result.status.success(),
                    valid,
                    "{name} workers={workers} check={check}: {stderr}\n{source}"
                );
                if !valid || check {
                    assert!(!output.exists(), "{name}: published executable");
                }
                if !valid {
                    assert!(!stderr.is_empty(), "{name}: missing diagnostic");
                } else if !check {
                    let bytes = fs::read(&output).unwrap();
                    assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                    let result = capture(&mut Command::new(output), &scratch, 5);
                    assert!(result.status.success(), "{name}: {result:?}");
                    assert_eq!(result.stdout, b"empty enum ok\n", "{name}");
                    assert!(result.stderr.is_empty(), "{name}: {result:?}");
                }
            }
        }
    }
}

#[test]
fn empty_enum_source_representation_and_check_graph_remain_nominal() {
    use justrust_engine::compiler::{self, frontend, ir};
    let text = "enum Void{}fn absurd(value:Void)->i32{match value{}}fn main(){}";
    let program = frontend::parse(text).unwrap();
    assert_eq!(program.enums.len(), 1);
    assert!(program.enums[0].variants.is_empty());
    let signatures = frontend::signature_table(&program).unwrap();
    let function = frontend::type_function_with_signatures(&program, 0, &signatures).unwrap();
    function.validate().unwrap();
    let ir::Ty::Enum(value) = &function.locals[function.params[0].0].ty else {
        panic!("empty enum was erased or replaced by a scalar/never type")
    };
    assert!(value.def().variants.is_empty());
    assert!(function.exprs.iter().any(|expression| {
        matches!(&expression.kind, ir::ExprKind::Match { arms, .. } if arms.is_empty())
            && expression.ty == ir::Ty::Never
    }));
    for workers in [1, 8] {
        let checked = compiler::compile_observed(text.into(), workers, true).unwrap();
        assert_eq!(checked.enums, 1);
        assert!(checked.executable.is_none());
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| key.kind != 103),
            "empty-enum CHECK demanded code generation"
        );
    }
}

#[test]
fn accepts_tuple_variants() {
    runs(
        r#"enum E{A(i32,bool),B} fn run()->i32{match E::A(7,true){E::A(n,b)=>if b{n}else{0},E::B=>0}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_braced_variants() {
    runs(
        r#"enum E{A{x:i32,y:i32},B} fn run()->i32{match (E::A{y:7,x:3}){E::A{x:a,y:b}=>a*10+b,E::B=>0}}
fn main() { if run() == 37 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_unit_and_empty_forms() {
    runs(
        r#"enum E{A,B(),C{}} fn score(e:E)->i32{match e{E::A=>1,E::B()=>2,E::C{}=>3}} fn run()->i32{score(E::A{})+score(E::B())+score(E::C{})}
fn main() { if run() == 6 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_positional_tuple_rest() {
    runs(
        r#"enum E{A(i32,i32,i32,i32),B} fn run()->i32{match E::A(1,2,3,4){E::A(a,..,d)=>a*10+d,E::B=>0}}
fn main() { if run() == 14 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_braced_rest_shorthand() {
    runs(
        r#"enum E{A{x:i32,y:i32,z:i32},B} fn run()->i32{match (E::A{x:3,y:7,z:9}){E::A{x, y:renamed,..}=>x*10+renamed,E::B=>0}}
fn main() { if run() == 37 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_singlevariant_tuple_let() {
    runs(
        r#"enum E{A(i32,i32)} fn run()->i32{let E::A(a,b)=E::A(3,7);a*10+b}
fn main() { if run() == 37 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_singlevariant_braced_let() {
    runs(
        r#"enum E{A{x:i32,y:i32}} fn run()->i32{let E::A{x, y:mut b}=E::A{x:3,y:4};b=b+3;x*10+b}
fn main() { if run() == 37 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_nested_tuple_struct_payload() {
    runs(
        r#"struct S{n:i32} enum E{A((i32,S)),B} fn run()->i32{match E::A((3,S{n:7})){E::A((a,S{n:b}))=>a*10+b,E::B=>0}}
fn main() { if run() == 37 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_nested_enum_payload_value() {
    runs(
        r#"enum I{A(i32),B} enum E{A(I),B} fn run()->i32{match E::A(I::A(7)){E::A(inner)=>match inner{I::A(n)=>n,I::B=>0},E::B=>0}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_copy_payload_repeated() {
    runs(
        r#"enum E{A(i32),B} fn score(e:E)->i32{match e{E::A(n)=>n,E::B=>0}} fn run()->i32{let e=E::A(7);let n=match e{E::A(n)=>n,E::B=>0};n+score(e)}
fn main() { if run() == 14 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_singleton_disjoint_moves() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S)} fn run()->i32{let e=E::A(S{n:3},S{n:7});let E::A(a,_)=e;let E::A(_,b)=e;take(a)+take(b)}
fn main() { if run() == 10 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_singleton_braced_disjoint_moves() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A{x:S,y:S}} fn run()->i32{let e=E::A{x:S{n:3},y:S{n:7}};let E::A{x:a,..}=e;let E::A{y:b,..}=e;take(a)+take(b)}
fn main() { if run() == 10 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_selected_payload_fields() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{match E::A(S{n:3},S{n:7}){E::A(a,b)=>take(a)+take(b),E::B=>0}}
fn main() { if run() == 10 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_after_whole_move() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{match e{E::A(a)=>take(a),E::B=>0}} fn run()->i32{let e=E::A(S{n:3});let a=consume(e);a+match e{_=>7}}
fn main() { if run() == 10 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_singleton_payload_wildcard_after_move() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S)} fn consume(e:E)->i32{let E::A(a)=e;take(a)} fn run()->i32{let e=E::A(S{n:3});let a=consume(e);a+match e{E::A(_)=>7}}
fn main() { if run() == 10 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_whole_binding_payload() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{match e{E::A(a)=>take(a),E::B=>0}} fn run()->i32{let e=E::A(S{n:7});match e{x=>consume(x)}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_constructor_source_order() {
    runs(
        r#"enum E{A{x:i32,y:i32}} fn first()->i32{println!("first");3} fn second()->i32{println!("second");7} fn run()->i32{let e=E::A{y:first(),x:second()};let E::A{x,y}=e;x*10+y}
fn main() { if run() == 73 { println!("payload oracle ok"); } }
"#,
        "first\nsecond\npayload oracle ok\n",
    );
}

#[test]
fn accepts_large_return_argument_abi() {
    runs(
        r#"enum E{A(i32,i32,i32,i32,i32,i32,i32,i32),B} fn make()->E{E::A(1,2,3,4,5,6,7,8)} fn sum(e:E)->i32{match e{E::A(a,b,c,d,e,f,g,h)=>a+b+c+d+e+f+g+h,E::B=>0}} fn run()->i32{sum(make())}
fn main() { if run() == 36 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_recursive_function_abi() {
    runs(
        r#"enum E{A(i32,i32),B} fn recur(e:E,n:i32)->i32{if n==0{match e{E::A(a,b)=>a+b,E::B=>0}}else{recur(e,n-1)}} fn run()->i32{recur(E::A(3,7),4)}
fn main() { if run() == 10 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_nominal_alias_import() {
    runs(
        r#"enum E{A(i32),B} use E as Alias; use E::A as Make; fn run()->i32{match Make(7){Alias::A(n)=>n,Alias::B=>0}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_public_module_payload() {
    runs(
        r#"mod m{pub enum E{A{n:i32},B} pub fn make()->E{E::A{n:7}}} fn run()->i32{match m::make(){m::E::A{n}=>n,m::E::B=>0}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_zero_sized_payload() {
    runs(
        r#"enum E{A((),bool),B} fn run()->i32{match E::A((),true){E::A((),b)=>if b{7}else{0},E::B=>0}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn rejects_invalid_partial_multivariant_rematch() {
    // Stock rejects this fixture with E0382.
    rejects(
        "partial_multivariant_rematch",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{let e=E::A(S{n:3},S{n:7});let n=match e{E::A(a,_)=>take(a),E::B=>0};n+match e{E::A(_,b)=>take(b),E::B=>0}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_whole_after_partial() {
    // Stock rejects this fixture with E0382.
    rejects(
        "whole_after_partial",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S)} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(S{n:3},S{n:7});let E::A(a,_)=e;take(a)+consume(e)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_same_payload_twice() {
    // Stock rejects this fixture with E0382.
    rejects(
        "same_payload_twice",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S)} fn run()->i32{let e=E::A(S{n:3});let E::A(a)=e;let E::A(b)=e;take(a)+take(b)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_whole_binding_moves() {
    // Stock rejects this fixture with E0382.
    rejects(
        "whole_binding_moves",
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S),B} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(S{n:7});let a=match e{x=>consume(x)};a+consume(e)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_enum_scalar_payload_noncopy() {
    // Stock rejects this fixture with E0382.
    rejects(
        "enum_scalar_payload_noncopy",
        r#"enum E{A(i32),B} fn consume(e:E)->i32{0} fn run()->i32{let e=E::A(7);let a=consume(e);a+consume(e)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_wrong_payload_type() {
    // Stock rejects this fixture with E0308.
    rejects(
        "wrong_payload_type",
        r#"enum E{A(i32)} fn run()->i32{let e=E::A(true);0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_wrong_nominal_enum() {
    // Stock rejects this fixture with E0308.
    rejects(
        "wrong_nominal_enum",
        r#"enum E{A(i32)} enum F{A(i32)} fn consume(e:E)->i32{0} fn run()->i32{consume(F::A(7))}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_tuple_ctor_arity() {
    // Stock rejects this fixture with E0061.
    rejects(
        "tuple_ctor_arity",
        r#"enum E{A(i32,i32)} fn run()->i32{let e=E::A(7);0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_tuple_pattern_arity() {
    // Stock rejects this fixture with E0023.
    rejects(
        "tuple_pattern_arity",
        r#"enum E{A(i32,i32)} fn run()->i32{match E::A(3,7){E::A(a)=>a}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_missing_braced_field() {
    // Stock rejects this fixture with E0063.
    rejects(
        "missing_braced_field",
        r#"enum E{A{x:i32,y:i32}} fn run()->i32{let e=E::A{x:3};0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unknown_braced_field() {
    // Stock rejects this fixture with E0559.
    rejects(
        "unknown_braced_field",
        r#"enum E{A{x:i32}} fn run()->i32{let e=E::A{x:3,y:7};0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_refutable_variant_let() {
    // Stock rejects this fixture with E0005.
    rejects(
        "refutable_variant_let",
        r#"enum E{A(i32),B} fn run()->i32{let E::A(n)=E::A(7);n}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_private_enum_payload() {
    // Stock rejects this fixture with E0603.
    rejects(
        "private_enum_payload",
        r#"mod m{enum E{A(i32)}} fn run()->i32{let e=m::E::A(7);0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_direct_recursive_enum() {
    // Stock rejects this fixture with E0072.
    rejects(
        "direct_recursive_enum",
        r#"enum E{A(E)} fn run()->i32{0}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_indirect_recursive_enum() {
    // Stock rejects this fixture with E0072.
    rejects(
        "indirect_recursive_enum",
        r#"struct S{e:E} enum E{A(S)} fn run()->i32{0}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_nested_refutable_payload_product_coverage() {
    // Exact historical stock-valid fixture, now admitted by checked usefulness.
    runs(
        r#"enum I{A,B} enum E{X(I)} fn run()->i32{match E::X(I::A){E::X(I::A)=>3,E::X(I::B)=>7}}
fn main() { if run() == 3 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_payload_guard_with_unused_invalid_condition_control() {
    payload_transition(
        r#"enum E{A(i32),B} fn run()->i32{match E::A(7){E::A(n) if n>0=>n,_=>0}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "enum E{A(i32),B}fn unused()->i32{match E::A(7){E::A(n) if n=>n,_=>0}}fn main(){}",
    );
}

#[test]
fn accepts_payload_or_pattern_with_unused_invalid_result_control() {
    payload_transition(
        r#"enum E{A(i32),B(i32)} fn run()->i32{match E::A(7){E::A(n)|E::B(n)=>n}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "enum E{A(i32),B(i32)}fn unused()->bool{match E::A(7){E::A(n)|E::B(n)=>n}}fn main(){}",
    );
}

#[test]
fn accepts_payload_reference_binding() {
    runs(
        r#"enum E{A(i32)} fn run()->i32{let e=E::A(7);match e{E::A(ref n)=>*n}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_payload_drop_with_unused_invalid_body_control() {
    payload_transition(
        r#"struct S{n:i32} impl Drop for S{fn drop(&mut self){}} enum E{A(S)} fn run()->i32{let e=E::A(S{n:7});match e{E::A(s)=>s.n}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
        "struct S{n:i32}impl Drop for S{fn drop(&mut self){let invalid:u8=true;}}enum E{A(S)}fn unused()->i32{let e=E::A(S{n:7});match e{E::A(s)=>s.n}}fn main(){}",
    );
}

#[test]
fn accepts_payload_binding_snapshot() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum E{A(S,S),B} fn run()->i32{let mut e=E::A(S{n:3},S{n:7});let n=match e{E::A(a,b)=>{e=E::B;take(a)+take(b)},E::B=>0};n+match e{E::A(_,_)=>100,E::B=>1}}
fn main() { if run() == 11 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}

#[test]
fn accepts_nested_singleton_payload_pattern() {
    runs(
        r#"struct S{n:i32} fn take(s:S)->i32{s.n} enum Inner{Item(S,S)} enum Outer{Wrap(Inner),Empty} fn run()->i32{match Outer::Wrap(Inner::Item(S{n:3},S{n:7})){Outer::Wrap(Inner::Item(S{n:a},b))=>a+take(b),Outer::Empty=>0}}
fn main() { if run() == 10 { println!("payload oracle ok"); } }
"#,
        "payload oracle ok\n",
    );
}
