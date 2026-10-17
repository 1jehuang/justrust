//! Fieldless enum and match native acceptance for the bounded milestone.
//! All 59 exact source fixtures were classified with stock rustc 1.98.1, edition 2024.
//! The promoted historical alias fixture also reruns its executable stock oracle.
//! Stock is an external scratch oracle only, never a native compiler dependency.
//! Payloads and other legal-unsupported cases are boundaries, not invalid Rust.
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
            let path = base.join(format!("jr-match-{}-{epoch}-{id}", std::process::id()));
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

#[test]
fn accepts_variants() {
    runs(
        r#"enum E { A, B, C } fn run()->i32 { let e=E::B; match e { E::A=>1, E::B=>2, E::C=>3 } }
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_repeat_noncopy() {
    runs(
        r#"enum E { A, B, C } fn run()->i32 { let e=E::B; let a=match e { E::A=>1, _=>2 }; let b=match e { _=>3 }; a+b+match e { E::A=>4,E::B=>5,E::C=>6 } }
fn main() { if run() == 10 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_binding_moves() {
    runs(
        r#"enum E { A, B, C } fn score(e:E)->i32 { match e { E::A=>1,_=>2 } } fn run()->i32 { let e=E::B; match e { x=>score(x) } }
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_pass_return() {
    runs(
        r#"enum E { A, B, C } fn make()->E { E::C } fn score(e:E)->i32 { match e { E::A=>1,E::B=>2,E::C=>3 } } fn run()->i32 { score(make()) }
fn main() { if run() == 3 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_reinitialize() {
    runs(
        r#"enum E { A, B, C } fn run()->i32 { let mut e=E::A; let x=match e { E::A=>1,_=>2 }; e=E::C; x+match e { E::C=>3,_=>4 } }
fn main() { if run() == 4 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_arm_blocks() {
    runs(
        r#"enum E { A, B, C } fn run()->i32 { match E::B { E::A=>{let n=1;n}, E::B=>{let n=2;n+3}, E::C=>6 } }
fn main() { if run() == 5 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_nested() {
    runs(
        r#"enum E { A, B, C } fn run()->i32 { match E::A { E::A=>match E::C { E::C=>7,_=>8 },_=>9 } }
fn main() { if run() == 7 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_projection_nonconsume() {
    runs(
        r#"enum E { A, B, C } struct Pair { e:E,n:i32 } fn run()->i32 { let p=Pair{e:E::B,n:4}; let x=match p.e { E::B=>2,_=>3 }; x+match p.e { _=>p.n } }
fn main() { if run() == 6 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn rejects_invalid_not_exhaustive() {
    // Stock rejects this fixture with E0004.
    rejects(
        "not_exhaustive",
        r#"enum E { A, B, C } fn run()->i32 { match E::A { E::A=>1,E::B=>2 } }
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_arm_types() {
    // Stock rejects this fixture with E0308.
    rejects(
        "arm_types",
        r#"enum E { A, B, C } fn run()->i32 { match E::A { E::A=>1,_=>true } }
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_wrong_enum() {
    // Stock rejects this fixture with E0308.
    rejects(
        "wrong_enum",
        r#"enum E { A, B, C } enum F { A } fn run()->i32 { match E::A { F::A=>1,_=>2 } }
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unknown_variant() {
    // Stock rejects this fixture with E0599.
    rejects(
        "unknown_variant",
        r#"enum E { A, B, C } fn run()->i32 { match E::A { E::Missing=>1,_=>2 } }
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_binding_use_after_move() {
    // Stock rejects this fixture with E0382.
    rejects(
        "binding_use_after_move",
        r#"enum E { A, B, C } fn take(e:E)->i32 { 1 } fn run()->i32 { let e=E::A; let n=match e { x=>take(x) }; n+take(e) }
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_plain_move() {
    // Stock rejects this fixture with E0382.
    rejects(
        "plain_move",
        r#"enum E { A, B, C } fn take(e:E)->i32 { 1 } fn run()->i32 { let e=E::A; let x=e; take(x)+take(e) }
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_arm_scope() {
    // Stock rejects this fixture with E0425.
    rejects(
        "arm_scope",
        r#"enum E { A, B, C } fn run()->i32 { let n=match E::A { x=>1 }; n+match x { _=>2 } }
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_duplicate_variant() {
    // Stock rejects this fixture with E0428.
    rejects(
        "duplicate_variant",
        r#"enum E { A,A } fn run()->i32 { 1 }
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_historical_payload_fixture() {
    // The original legal-unsupported source is now supported, unchanged.
    runs(
        r#"enum E { A(i32),B } fn run()->i32 { match E::A(4) { E::A(n)=>n,E::B=>0 } }
fn main() { if run() == 4 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_moved() {
    runs(
        r#"enum E { A,B } fn take(e:E)->i32{1} fn run()->i32{let e=E::A; let n=take(e); n+match e {_=>2}}
fn main() { if run() == 3 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_guard() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "guard",
        r#"enum E { A, B, C } fn run()->i32 { match E::A { E::A if true=>1,_=>2 } }
fn main() { if run() == 1 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_or_pattern() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "or_pattern",
        r#"enum E { A, B, C } fn run()->i32 { match E::A { E::A|E::B=>1,E::C=>2 } }
fn main() { if run() == 1 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_ref_binding() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "ref_binding",
        r#"enum E { A, B, C } fn run()->i32 { let e=E::A; match e { ref x=>1 } }
fn main() { if run() == 1 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_at_binding() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "at_binding",
        r#"enum E { A, B, C } fn run()->i32 { match E::A { x @ E::A=>1,_=>2 } }
fn main() { if run() == 1 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_explicit_discriminant() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "explicit_discriminant",
        r#"enum E { A=5,B=9 } fn run()->i32 { match E::B { E::A=>1,E::B=>2 } }
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn accepts_unreachable_arm() {
    runs(
        r#"enum E { A, B, C } fn run()->i32 { match E::A { _=>1,E::A=>2 } }
fn main() { if run() == 1 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_bool_coverage() {
    runs(
        r#"fn run()->i32{match true{true=>4,false=>5}}
fn main() { if run() == 4 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_unit_coverage() {
    runs(
        r#"fn run()->i32{match (){()=>6}}
fn main() { if run() == 6 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_integer_catchall() {
    runs(
        r#"fn score(x:i32)->i32{match x{0=>7,9=>8,_=>3}} fn run()->i32{score(0)+score(9)+score(-1)}
fn main() { if run() == 18 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_shadow_binding() {
    runs(
        r#"fn run()->i32{let n=7; let x=match 4{n=>n+1}; x+n}
fn main() { if run() == 12 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_mutable_binding() {
    runs(
        r#"fn run()->i32{match 4{mut n=>{n=n+3;n}}}
fn main() { if run() == 7 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_tuple_result() {
    runs(
        r#"fn run()->i32{let p=match true{true=>(3,4),false=>(5,6)}; p.0*10+p.1}
fn main() { if run() == 34 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_struct_result() {
    runs(
        r#"struct S{n:i32} fn run()->i32{let s=match false{true=>S{n:3},false=>S{n:8}};s.n}
fn main() { if run() == 8 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_enum_result() {
    runs(
        r#"enum E { A, B, C } fn run()->i32{let e=match true{true=>E::B,false=>E::C};match e{E::A=>1,E::B=>2,E::C=>3}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_uninitialized() {
    runs(
        r#"enum E { A, B, C } fn run()->i32{let e:E;match e{_=>7}}
fn main() { if run() == 7 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_scrutinee_once() {
    runs(
        r#"enum E { A, B, C } fn make()->E{println!("once");E::B} fn run()->i32{match make(){E::A=>1,E::B=>2,E::C=>3}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "once\nmatch oracle ok\n",
    );
}

#[test]
fn accepts_wildcard_effect_once() {
    runs(
        r#"fn make()->i32{println!("once");4} fn run()->i32{match make(){_=>7}}
fn main() { if run() == 7 { println!("match oracle ok"); } }
"#,
        "once\nmatch oracle ok\n",
    );
}

#[test]
fn accepts_duplicate_arm_first() {
    runs(
        r#"enum E { A, B, C } fn run()->i32{match E::A{E::A=>3,E::A=>4,_=>5}}
fn main() { if run() == 3 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_historical_grouped_import() {
    runs(
        r#"enum E { A, B, C } use E::{A,B,C}; fn run()->i32{match B{A=>1,B=>2,C=>3}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_historical_type_alias() {
    // Preserve this exact historical source and rerun its executable stock oracle.
    let source = r#"enum E { A, B, C } type Alias=E; fn run()->i32{match Alias::B{Alias::A=>1,Alias::B=>2,Alias::C=>3}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#;
    let scratch = Scratch::new();
    let input = scratch.source(source);
    let output = scratch.path("stock-alias");
    let result = capture(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg(&input)
            .arg("-o")
            .arg(&output),
        &scratch,
        20,
    );
    assert!(result.status.success(), "stock alias: {result:?}");
    let runtime = capture(&mut Command::new(output), &scratch, 5);
    assert!(runtime.status.success(), "stock runtime: {runtime:?}");
    assert_eq!(runtime.stdout, b"match oracle ok\n");
    assert!(runtime.stderr.is_empty());
    runs(source, "match oracle ok\n");
}

#[test]
fn accepts_bare_binder() {
    runs(
        r#"enum E{A,B} fn score(e:E)->i32{match e{E::A=>1,E::B=>2}} fn run()->i32{match E::B{e=>score(e)}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn rejects_invalid_variant_after_move() {
    // Stock rejects this fixture with E0382.
    rejects(
        "variant_after_move",
        r#"enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{E::A=>1,_=>2}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_variant_uninitialized() {
    // Stock rejects this fixture with E0381.
    rejects(
        "variant_uninitialized",
        r#"enum E { A, B, C } fn run()->i32{let e:E;match e{E::A=>1,_=>2}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_arm_join_move() {
    // Stock rejects this fixture with E0382.
    rejects(
        "arm_join_move",
        r#"enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=match true{true=>take(e),false=>0};n+take(e)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_unreachable_still_typed() {
    // Stock rejects this fixture with E0308.
    rejects(
        "unreachable_still_typed",
        r#"enum E { A, B, C } fn run()->i32{match E::A{_=>1,E::A=>false}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_integer_no_catchall() {
    // Stock rejects this fixture with E0004.
    rejects(
        "integer_no_catchall",
        r#"fn run()->i32{match 1{1=>2}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_bool_not_exhaustive() {
    // Stock rejects this fixture with E0004.
    rejects(
        "bool_not_exhaustive",
        r#"fn run()->i32{match true{true=>2}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_empty_enum() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "empty_enum",
        r#"enum E{} fn run()->i32{3}
fn main() { if run() == 3 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn accepts_inferred_generic_enum() {
    // Source bytes unchanged, now independently stock/native runtime verified.
    runs(
        r#"enum E<T>{A(T)} fn run()->i32{match E::A(3){E::A(n)=>n}}
fn main() { if run() == 3 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_range_pattern() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "range_pattern",
        r#"fn run()->i32{match 2{1..=3=>4,_=>5}}
fn main() { if run() == 4 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_nested_refutable() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "nested_refutable",
        r#"enum E{A(bool)} fn run()->i32{match E::A(true){E::A(true)=>1,E::A(false)=>2}}
fn main() { if run() == 1 { println!("match oracle ok"); } }
"#,
    );
}

#[test]
fn accepts_module_namespace() {
    runs(
        r#"mod m { pub enum E{A,B} pub fn make()->E{E::B} } fn run()->i32{match m::make(){m::E::A=>1,m::E::B=>2}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn rejects_invalid_private_enum() {
    // Stock rejects this fixture with E0603.
    rejects(
        "private_enum",
        r#"mod m { enum E{A} } fn run()->i32{match m::E::A{_=>1}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_enum_not_value() {
    // Stock rejects this fixture with E0423.
    rejects(
        "enum_not_value",
        r#"enum E { A, B, C } fn run()->i32{let x=E;1}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_dead_move_join() {
    // Stock rejects this fixture with E0382.
    rejects(
        "dead_move_join",
        r#"enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=match true{_=>0,true=>take(e)};n+take(e)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_moved_wildcard_then_variant() {
    // Stock rejects this fixture with E0382.
    rejects(
        "moved_wildcard_then_variant",
        r#"enum E { A, B, C } fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{_=>2,E::A=>3}}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_singleton_after_move() {
    runs(
        r#"enum E{A} fn take(e:E)->i32{1} fn run()->i32{let e=E::A;let n=take(e);n+match e{E::A=>2}}
fn main() { if run() == 3 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn rejects_invalid_dead_break_type() {
    // Stock rejects this fixture with E0308.
    rejects(
        "dead_break_type",
        r#"fn run()->i32{loop{match true{_=>break 1,true=>break false}}}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_imported_variant_parameter() {
    // Stock rejects this fixture with E0005.
    rejects(
        "imported_variant_parameter",
        r#"enum E { A, B, C } use E::A; fn score(A:E)->i32{1} fn run()->i32{score(E::A)}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_imported_variant() {
    runs(
        r#"enum E { A, B, C } use E::A; use E::B; use E::C; fn run()->i32{match B{A=>1,B=>2,C=>3}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}

#[test]
fn accepts_alias_variant() {
    runs(
        r#"enum E { A, B, C } use E as Alias; fn run()->i32{match Alias::B{Alias::A=>1,Alias::B=>2,Alias::C=>3}}
fn main() { if run() == 2 { println!("match oracle ok"); } }
"#,
        "match oracle ok\n",
    );
}
