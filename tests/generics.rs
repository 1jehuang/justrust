//! Universal generic templates, unchanged frozen stock28 corpus.
//! Resource errors are separate from invalid Rust and unsupported valid sources.
//! Stock expanding-instance CHECK succeeds, BUILD hits recursion limit; native
//! CHECK/BUILD must reject usefully with exact exit1 and a generic limit diagnostic.
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
            let path = base.join(format!("jr-generics-{}-{epoch}-{id}", std::process::id()));
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
fn runs_with_exit(source: &str, expected: &str, expected_exit: i32) {
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
        assert_eq!(
            result.status.code(),
            Some(expected_exit),
            "runtime status failure: {result:?}\n{source}"
        );
        assert_eq!(
            result.stdout,
            expected.as_bytes(),
            "workers={workers}\n{source}"
        );
        if expected_exit == 0 {
            assert!(
                result.stderr.is_empty(),
                "unexpected runtime stderr: {result:?}"
            );
        }
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

fn runs(source: &str, expected: &str) {
    runs_with_exit(source, expected, 0);
}

fn rejects_resource(source: &str) {
    rejects_resource_modes(source, &[true, false]);
}
fn rejects_resource_build(source: &str) {
    rejects_resource_modes(source, &[false]);
}
fn rejects_resource_modes(source: &str, modes: &[bool]) {
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        for &check in modes {
            let result = capture(&mut compiler(&input, &output, workers, check), &scratch, 20);
            assert_eq!(
                result.status.code(),
                Some(1),
                "resource failure workers={workers}, check={check}: {result:?}"
            );
            let diagnostic = String::from_utf8_lossy(&result.stderr).to_ascii_lowercase();
            assert!(
                diagnostic.contains("generic") && diagnostic.contains("limit"),
                "expected bounded generic limit diagnostic, not arbitrary rejection: {result:?}"
            );
            for forbidden in [
                "panicked at",
                "parse error",
                "unexpected token",
                "unsupported syntax",
            ] {
                assert!(
                    !diagnostic.contains(forbidden),
                    "not a resource guard ({forbidden}): {result:?}"
                );
            }
            assert!(!output.exists(), "resource rejection published executable");
        }
    }
}

#[test]
fn accepts_scalar_identity() {
    runs(
        r#"fn id<T>(x:T)->T{x}fn run()->i32{id::<i32>(7)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_identity() {
    runs(
        r#"struct S{x:i32}fn id<T>(x:T)->T{x}fn run()->i32{let s=id::<S>(S{x:7});s.x}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_scalar_wrapper() {
    runs(
        r#"struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{wrap::<i32>(7).value}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_wrapper() {
    runs(
        r#"struct S{x:i32}struct Wrap<T>{value:T}fn wrap<T>(x:T)->Wrap<T>{Wrap::<T>{value:x}}fn run()->i32{let w=wrap::<S>(S{x:7});w.value.x}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_enum_variant_shapes() {
    runs(
        r#"enum E<T>{Unit,Tuple(T),Braced{value:T}}fn get<T>(e:E<T>,fallback:T)->T{match e{E::Unit=>fallback,E::Tuple(v)=>v,E::Braced{value}=>value}}fn run()->i32{let a=get::<i32>(E::<i32>::Unit,1);let b=get::<i32>(E::<i32>::Tuple(2),0);let c=get::<i32>(E::<i32>::Braced{value:4},0);a+b+c}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_tuple_payload_match() {
    runs(
        r#"enum E<T>{A((T,i32)),B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A((v,_))=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A((7,2)),0)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_braced_payload_match() {
    runs(
        r#"enum E<T>{A{value:T,n:i32},B}fn extract<T>(e:E<T>,other:T)->T{match e{E::A{value:v,..}=>v,E::B=>other}}fn run()->i32{extract::<i32>(E::<i32>::A{value:7,n:2},0)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_struct_disjoint_partial_moves() {
    runs(
        r#"struct Pair<T>{a:T,b:T}fn split<T>(p:Pair<T>)->(T,T){let a=p.a;let b=p.b;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>{a:3,b:4});p.0+p.1}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_singleton_enum_disjoint_moves() {
    runs(
        r#"enum Pair<T>{A{a:T,b:T}}fn split<T>(p:Pair<T>)->(T,T){let Pair::A{a,..}=p;let Pair::A{b,..}=p;(a,b)}fn run()->i32{let p=split::<i32>(Pair::<i32>::A{a:3,b:4});p.0+p.1}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_explicit_generic_forwarding() {
    runs(
        r#"fn id<T>(x:T)->T{x}fn forward<U>(x:U)->U{id::<U>(x)}fn run()->i32{forward::<i32>(7)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_exact_self_recursion() {
    runs(
        r#"fn rec<T>(x:T,n:i32)->T{if n==0{x}else{rec::<T>(x,n-1)}}fn run()->i32{rec::<i32>(7,4)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_exact_mutual_recursion() {
    runs(
        r#"fn a<T>(x:T,n:i32)->T{if n==0{x}else{b::<T>(x,n-1)}}fn b<U>(x:U,n:i32)->U{if n==0{x}else{a::<U>(x,n-1)}}fn run()->i32{a::<i32>(7,4)}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_multiple_nominal_specializations() {
    runs(
        r#"struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let a=id::<Wrap<i32>>(Wrap::<i32>{value:7});let b=id::<Wrap<u64>>(Wrap::<u64>{value:18446744073709551615u64});if b.value==18446744073709551615u64{a.value}else{0}}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_distinct_nominal_generic_owners() {
    runs(
        r#"struct A<T>{value:T}struct B<T>{value:T}fn run()->i32{let a=A::<i32>{value:3};let b=B::<i32>{value:4};a.value+b.value}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_nested_reference_free_substitution() {
    runs(
        r#"struct Wrap<T>{value:T}enum E<T>{A(T)}fn id<T>(x:T)->T{x}fn run()->i32{let w=id::<Wrap<E<([i32;1],i32)>>>(Wrap::<E<([i32;1],i32)>>{value:E::<([i32;1],i32)>::A(([3],4))});match w.value{E::A((a,b))=>a[0]+b}}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn accepts_valid_unused_template() {
    runs(
        r#"fn unused<T>(x:T)->T{x}fn run()->i32{7}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_invalid_unused_duplicate() {
    rejects(
        "negative: unused_duplicate",
        r#"fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_duplicate_with_copy_caller() {
    rejects(
        "negative: duplicate_with_copy_caller",
        r#"fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{let p=bad::<i32>(7);p.0}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_generic_operator_copy_caller() {
    rejects(
        "negative: generic_operator_copy_caller",
        r#"fn bad<T>(x:T)->T{x+1}fn run()->i32{bad::<i32>(7)}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_generic_return_mismatch() {
    rejects(
        "negative: generic_return_mismatch",
        r#"fn bad<T>(x:T)->T{7i32}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_generic_field_reuse() {
    rejects(
        "negative: generic_field_reuse",
        r#"struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->(T,T){let a=p.a;(a,p.a)}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_whole_after_generic_partial_move() {
    rejects(
        "negative: whole_after_generic_partial_move",
        r#"struct Pair<T>{a:T,b:T}fn bad<T>(p:Pair<T>)->Pair<T>{let a=p.a;p}fn run()->i32{7}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_nominal_argument_mismatch() {
    rejects(
        "negative: nominal_argument_mismatch",
        r#"struct Wrap<T>{value:T}fn take(x:Wrap<i32>)->i32{x.value}fn run()->i32{take(Wrap::<u8>{value:7})}
fn main(){run();}"#,
    );
}

#[test]
fn rejects_invalid_wrong_generic_arity() {
    rejects(
        "negative: wrong_generic_arity",
        r#"fn id<T>(x:T)->T{x}fn run()->i32{id::<i32,u8>(7)}
fn main(){run();}"#,
    );
}

#[test]
fn runs_copy_bound_trait_dependency() {
    runs(
        r#"fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let p=dup::<i32>(7);p.0}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_legal_unsupported_direct_reference_substitution() {
    // Historical group name retained. The exact source now has native parity evidence.
    runs(
        r#"fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let r=id::<&i32>(&x);*r}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_escaping_hidden_reference_substitution() {
    // Stock rustc: E0515, the reference to local `x` escapes through `id`.
    rejects(
        "escaping hidden reference",
        r#"struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->&'static i32{let x=7;let w=id::<Wrap<&i32>>(Wrap::<&i32>{value:&x});w.value}
fn main(){if *run()==7{println!("generic oracle ok");}}"#,
    );
}

#[test]
fn accepts_hidden_reference_substitution() {
    // Valid Rust: stock rustc builds and prints the oracle. Per-instance
    // INSTANCE planning now admits it; the escaping variant stays rejected.
    runs(
        r#"struct Wrap<T>{value:T}fn id<T>(x:T)->T{x}fn run()->i32{let x=7;let w=id::<Wrap<&i32>>(Wrap::<&i32>{value:&x});*w.value}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
        "generic oracle ok\n",
    );
}

#[test]
fn rejects_resource_expanding_recursive_instances() {
    // Stock rustc accepts this under `--emit=metadata` (check) and rejects it
    // only while instantiating for codegen ("reached the recursion limit while
    // instantiating"). CHECK must therefore accept, BUILD must reject.
    let source = r#"fn grow<T>(x:T){grow::<(T,)>((x,));}fn run()->i32{grow::<i32>(1);7}
fn main(){if run()==7{println!("generic oracle ok");}}"#;
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        let result = capture(&mut compiler(&input, &output, workers, true), &scratch, 20);
        assert_eq!(
            result.status.code(),
            Some(0),
            "stock check accepts: {result:?}"
        );
    }
    rejects_resource_build(
        r#"fn grow<T>(x:T){grow::<(T,)>((x,));}fn run()->i32{grow::<i32>(1);7}
fn main(){if run()==7{println!("generic oracle ok");}}"#,
    );
}

#[test]
fn generic_body_move_error_is_attributed_to_generic_item() {
    // Stock rustc rejects with E0382 in `twice`, for every instantiation.
    let source = r#"fn twice<T>(x:T)->(T,T){let a=x;let b=x;(a,b)}
fn main(){let _=twice::<i32>(1);let _=twice::<String>(String::new());}"#;
    rejects("generic move error", source);
    for workers in [1, 8] {
        let scratch = Scratch::new();
        let input = scratch.source(source);
        let output = scratch.path("must-not-exist");
        let result = capture(&mut compiler(&input, &output, workers, true), &scratch, 20);
        let stderr = String::from_utf8_lossy(&result.stderr);
        assert!(
            stderr.contains("twice"),
            "not attributed to generic item: {stderr}"
        );
        assert!(
            !stderr.contains(&usize::MAX.to_string()),
            "sentinel identity leaked: {stderr}"
        );
    }
}
