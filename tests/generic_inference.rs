//! Frozen exact28 first-order generic inference acceptance corpus.
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
                "jr-generic-inference-{}-{epoch}-{id}",
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
                result.status.code() == Some(1),
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

#[test]
fn accepts_later_known_integer_width() {
    runs(
        r###"fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(1,2u8);if x==2u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_hard_result_identity_context() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn run()->i32{let x:u8=id(7);if x==7u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_unsuffixed_identity_default() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn run()->i32{let x=id(7);let y=id(-((1)));let z=id(!0);if x==7 && y == -1 && z == -1{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_tuple_array_structural_evidence() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn run()->i32{let p=id((3u8,4u16));let a=id([3u8,4u8]);let r=id([7u16;1]);let empty:[u8;0]=id([]);if p.0==3u8 && p.1==4u16 && a[1]==4u8 && r[0]==7u16{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_known_call_and_cast_evidence() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn known()->u8{7u8}fn run()->i32{let a=id(known());let b=id(id::<u16>(7u16));let c=id(7i32 as u64);let d=id(256) as u8;if a==7u8 && b==7u16 && c==7u64 && d==0u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_inferred_struct_constructors() {
    runs(
        r###"struct Wrap<T>{value:T}fn run()->i32{let a=Wrap{value:3u8};let b:Wrap<u64>=Wrap{value:4};if a.value==3u8 && b.value==4u64{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_inferred_enum_constructors() {
    runs(
        r###"enum E<T>{None,Some(T),Named{value:T}}fn take<T>(e:E<T>,fallback:T)->T{match e{E::None=>fallback,E::Some(v)=>v,E::Named{value}=>value}}fn run()->i32{let a:E<u8>=E::None;let b=E::Some(3u8);let c=E::Named{value:4u8};if take(a,0u8)==0u8 && take(b,0u8)==3u8 && take(c,0u8)==4u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_rigid_generic_forwarding() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn forward<T>(x:T)->T{id(x)}fn run()->i32{forward(7)}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_rigid_exact_self_recursion() {
    runs(
        r###"fn rec<T>(x:T,n:i32)->T{if n==0{x}else{rec(x,n-1)}}fn run()->i32{let x=rec(7u8,4);if x==7u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_inferred_call_move() {
    runs(
        r###"fn id<T>(x:T)->T{x}struct S{x:i32}fn run()->i32{let s=id(S{x:7});s.x}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_constructor_disjoint_partial_moves() {
    runs(
        r###"struct S{x:i32}struct Pair<T>{a:T,b:T}fn run()->i32{let p=Pair{a:S{x:3},b:S{x:4}};let a=p.a;let b=p.b;a.x+b.x}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_argument_effects_once_ltr() {
    runs(
        r###"fn same<T>(a:T,b:T)->T{b}fn left(n:&mut i32)->u8{*n=*n*10+1;3u8}fn right(n:&mut i32)->u8{*n=*n*10+2;4u8}fn run()->i32{let mut n=0;let x=same(left(&mut n),right(&mut n));if n==12 && x==4u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_fully_explicit_compatibility() {
    runs(
        r###"fn id<T>(x:T)->T{x}struct W<T>{value:T}fn run()->i32{let a=id::<u8>(3u8);let b=id(4u8);let x=W::<u16>{value:3u16};let y=W{value:4u16};if a+b==7u8 && x.value+y.value==7u16{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_opaque_nested_with_other_evidence() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(id(1),2u8);if x==2u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_opaque_effect_argument_hard_context() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn same<T>(a:T,b:T)->T{b}fn run()->i32{let mut n=0;let x:u8=id({n=n+1;7});let y:u8=same(1,{n=n+1;let x=2u8;x});let z:u8=id(if true{7}else{0});if n==2 && x==7u8 && y==2u8 && z==7u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_symmetric_binary_peer_hints() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn run()->i32{let a=id(1)+2u8;let b=1+id(2u8);if a==3u8 && b==3u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn rejects_contradictory_known_argument_types() {
    rejects(
        "negative: contradictory_known_argument_types",
        r###"fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(1u8,2u16);7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_hard_result_vs_known_argument_conflict() {
    rejects(
        "negative: hard_result_vs_known_argument_conflict",
        r###"fn id<T>(x:T)->T{x}fn run()->i32{let x:u8=id(1u16);7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_noncopy_value_reused_after_inferred_move() {
    rejects(
        "negative: noncopy_value_reused_after_inferred_move",
        r###"fn id<T>(x:T)->T{x}struct S{x:i32}fn run()->i32{let s=S{x:7};let t=id(s);s.x}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_unused_universal_duplicate_body() {
    rejects(
        "negative: unused_universal_duplicate_body",
        r###"fn bad<T>(x:T)->(T,T){(x,x)}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_literal_out_of_range_after_later_width_constraint() {
    rejects(
        "negative: literal_out_of_range_after_later_width_constraint",
        r###"fn same<T>(a:T,b:T)->T{b}fn run()->i32{let x=same(256,2u8);7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_genuinely_unconstrained_unused_slot_ambiguity() {
    rejects(
        "negative: genuinely_unconstrained_unused_slot_ambiguity",
        r###"fn empty<T>(){}fn run()->i32{empty();7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_distinct_nominal_owner_conflict() {
    rejects(
        "negative: distinct_nominal_owner_conflict",
        r###"struct A<T>{value:T}struct B<T>{value:T}fn same<T>(a:A<T>,b:A<T>)->T{b.value}fn run()->i32{let a=A::<u8>{value:3};let b=B::<u8>{value:4};let x=same(a,b);7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_structural_tuple_or_array_shape_conflict() {
    rejects(
        "negative: structural_tuple_or_array_shape_conflict",
        r###"fn same<T>(a:(T,T),b:(T,T))->T{b.0}fn run()->i32{let x=same((1u8,2u8),(3u8,4u8,5u8));7}
fn main(){run();}
"###,
    );
}

#[test]
fn accepts_partial_explicit_type_hole() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn run()->i32{id::<_>(7)}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_local_future_use_required() {
    runs(
        r###"enum E<T>{None,Some(T)}fn take(e:E<i32>)->i32{match e{E::None=>7,E::Some(v)=>v}}fn run()->i32{let e=E::None;take(e)}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_nested_omitted_with_body_constraints() {
    runs(
        r###"fn id<T>(x:T)->T{x}fn run()->i32{let x=id(id(7));if x==7{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###,
        "inference oracle ok\n",
    );
}

#[test]
fn accepts_opaque_effect_argument_no_context() {
    let source = r###"fn same<T>(a:T,b:T)->T{b}fn run()->i32{let v=same(1,{let x=2u8;x});if v==2u8{7}else{0}}
fn main(){if run()==7{println!("inference oracle ok");}}
"###;
    // Preserve the exact former legal-unsupported source. Require a real stock
    // witness before checking native check/build/run with both worker counts.
    let scratch = Scratch::new();
    let input = scratch.source(source);
    let output = scratch.path("stock");
    let compiled = capture(
        Command::new("rustc")
            .arg("--edition=2024")
            .arg("-Cpanic=abort")
            .arg(input)
            .arg("-o")
            .arg(&output),
        &scratch,
        20,
    );
    assert!(compiled.status.success(), "stock: {compiled:?}");
    let result = capture(&mut Command::new(output), &scratch, 5);
    assert_eq!(result.status.code(), Some(0), "stock: {result:?}");
    assert_eq!(result.stdout, b"inference oracle ok\n");
    assert!(result.stderr.is_empty(), "stock: {result:?}");
    runs(source, "inference oracle ok\n");
}
