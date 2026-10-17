//! Payload enum native acceptance for the bounded tuple/braced/unit milestone.
//! All 44 exact fixtures were stock-classified with rustc 1.98.1, edition 2024.
//! Stock is an external scratch oracle only and is never invoked by these tests.
//! Guards, or-patterns, references and Drop remain boundaries.
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
fn rejects_legal_unsupported_payload_guard() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "payload_guard",
        r#"enum E{A(i32),B} fn run()->i32{match E::A(7){E::A(n) if n>0=>n,_=>0}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_payload_or_pattern() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "payload_or_pattern",
        r#"enum E{A(i32),B(i32)} fn run()->i32{match E::A(7){E::A(n)|E::B(n)=>n}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_payload_reference_binding() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "payload_reference_binding",
        r#"enum E{A(i32)} fn run()->i32{let e=E::A(7);match e{E::A(ref n)=>*n}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
    );
}

#[test]
fn rejects_legal_unsupported_payload_drop() {
    // Stock accepts and runs this fixture, outside this bounded native slice.
    rejects(
        "payload_drop",
        r#"struct S{n:i32} impl Drop for S{fn drop(&mut self){}} enum E{A(S)} fn run()->i32{let e=E::A(S{n:7});match e{E::A(s)=>s.n}}
fn main() { if run() == 7 { println!("payload oracle ok"); } }
"#,
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
