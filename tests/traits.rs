//! Frozen exact32 required-trait/static-dispatch acceptance corpus.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/stock_rustc.rs"]
mod unsafe_stock;

const UNSAFE_TRAIT_VALID: &[(&str, &str)] = &[
    (
        "marker",
        r#"struct S{x:i32}unsafe trait Marker{}unsafe impl Marker for S{}fn get<T:Marker>(x:T)->T{x}fn main(){let x=get(S{x:7});println!("{}",x.x);}"#,
    ),
    (
        "method",
        r#"struct S{x:i32}unsafe trait Read{fn value(&self)->i32;}unsafe impl Read for S{fn value(&self)->i32{self.x}}fn main(){let x=S{x:7};println!("{}",x.value());}"#,
    ),
    (
        "unused",
        r#"struct S{x:i32}unsafe trait Marker{}unsafe impl Marker for S{}fn main(){println!("7");}"#,
    ),
    (
        "module_alias",
        r#"mod origin{pub unsafe trait Marker{}}use origin::Marker as Alias;struct S{x:i32}unsafe impl Alias for S{}fn get<T:Alias>(x:T)->T{x}fn main(){let x=get(S{x:7});println!("{}",x.x);}"#,
    ),
    (
        "generic_impl",
        r#"unsafe trait Marker{}struct S<T>{x:T}unsafe impl<T> Marker for S<T>{}fn get<T:Marker>(x:T)->T{x}fn main(){let x=get(S{x:7});println!("{}",x.x);}"#,
    ),
    (
        "unsafe_supertrait",
        r#"unsafe trait Parent{}trait Child:Parent{}struct S{x:i32}unsafe impl Parent for S{}impl Child for S{}fn get<T:Child>(x:T)->T{x}fn main(){let x=get(S{x:7});println!("{}",x.x);}"#,
    ),
    (
        "local_sync_identity",
        r#"unsafe trait Sync{}struct S{x:i32}unsafe impl Sync for S{}fn get<T:Sync>(x:T)->T{x}fn main(){let x=get(S{x:7});println!("{}",x.x);}"#,
    ),
];
// Valid stock builtin auto-trait source. This is not native marker support.
const UNSAFE_TRAIT_BUILTIN_FRONTIER: &[(&str, &str)] = &[(
    "builtin_sync_frontier",
    r#"struct S{x:i32}unsafe impl Sync for S{}fn main(){println!("7");}"#,
)];
const UNSAFE_TRAIT_INVALID: &[(&str, &str)] = &[
    (
        "unused_missing_unsafe",
        r#"unsafe trait Marker{}struct S{x:i32}impl Marker for S{}fn main(){}"#,
    ),
    (
        "unused_unnecessary_unsafe",
        r#"trait Marker{}struct S{x:i32}unsafe impl Marker for S{}fn main(){}"#,
    ),
    (
        "unsafe_inherent",
        r#"struct S{x:i32}unsafe impl S{}fn main(){}"#,
    ),
    (
        "generic_missing_unsafe",
        r#"unsafe trait Marker{}struct S<T>{x:T}impl<T> Marker for S<T>{}fn main(){}"#,
    ),
    (
        "generic_unnecessary_unsafe",
        r#"trait Marker{}struct S<T>{x:T}unsafe impl<T> Marker for S<T>{}fn main(){}"#,
    ),
    (
        "builtin_copy_not_unsafe",
        r#"struct S{x:i32}unsafe impl core::marker::Copy for S{}fn main(){}"#,
    ),
    (
        "pub_unsafe_impl",
        r#"unsafe trait Marker{}struct S{x:i32}pub unsafe impl Marker for S{}fn main(){}"#,
    ),
];

#[test]
fn unsafe_trait_declarations_exact_stock_baseline() {
    for (valid, cases) in [
        (true, UNSAFE_TRAIT_VALID),
        (false, UNSAFE_TRAIT_INVALID),
        (true, UNSAFE_TRAIT_BUILTIN_FRONTIER),
    ] {
        for (label, source) in cases {
            let scratch = Scratch::new();
            let input = scratch.source(source);
            for check in [true, false] {
                let output = scratch.path(if check {
                    "stock.rmeta"
                } else {
                    "stock-program"
                });
                let mut command = Command::new(unsafe_stock::stock_rustc());
                command
                    .arg(&input)
                    .args(["--edition=2024", "--crate-name", "unsafe_trait_oracle"]);
                if check {
                    command.arg("--emit=metadata");
                }
                let result = capture(command.arg("-o").arg(&output), &scratch, 20);
                assert_eq!(
                    result.status.success(),
                    valid,
                    "{label} stock check={check}: {result:?}"
                );
                if valid && !check {
                    let result = capture(&mut Command::new(&output), &scratch, 5);
                    assert!(result.status.success(), "{label}: {result:?}");
                    assert_eq!(result.stdout, b"7\n", "{label}");
                }
            }
        }
    }
}

#[test]
fn unsafe_trait_declarations_preserve_native_safety_and_membership() {
    for (_, source) in UNSAFE_TRAIT_VALID {
        runs(source, "7\n");
    }
    for (label, source) in UNSAFE_TRAIT_INVALID {
        rejects(label, source);
    }
    for (label, source) in UNSAFE_TRAIT_BUILTIN_FRONTIER {
        rejects(label, source);
    }
}

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
            let path = base.join(format!("jr-traits-{}-{epoch}-{id}", std::process::id()));
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
    assert!(
        fs::metadata(&stdout).unwrap().len() <= 65536,
        "stdout capture exceeded64KiB"
    );
    assert!(
        fs::metadata(&stderr).unwrap().len() <= 65536,
        "stderr capture exceeded64KiB"
    );
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
fn accepts_canonical_float_clone_recipes() {
    runs(
        r#"fn duplicate<T:Clone>(x:&T)->T{x.clone()}fn main(){let a=1.5f32;let b=2.5f64;let x=duplicate::<f32>(&a);let y=duplicate::<f64>(&b);if x==1.5f32&&y==2.5f64{println!("float clone ok");}}"#,
        "float clone ok\n",
    );
}

#[test]
fn accepts_required_static_value_and_root_self_return() {
    runs(
        r###"struct S{x:i32}trait Take{fn make(x:i32)->Self;fn consume(self)->Self;fn value(self)->i32;}impl Take for S{fn make(x:i32)->Self{S{x}}fn consume(self)->Self{self}fn value(self)->i32{self.x}}fn run()->i32{<S as Take>::make(7).consume().value()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_impl_mut_self_trait_self_conformance() {
    runs(
        r###"struct S{x:i32}trait Add{fn add(self,n:i32)->i32;}impl Add for S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn run()->i32{S{x:3}.add(4)}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_multiple_required_methods_exact_signatures() {
    runs(
        r###"struct S{x:i32}trait Ops{fn first(self,n:i32)->i32;fn second(x:i32)->S;}impl Ops for S{fn first(self,n:i32)->i32{self.x+n}fn second(x:i32)->S{S{x}}}fn run()->i32{<S as Ops>::second(3).first(4)}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_distinct_nominal_implementors_same_trait() {
    runs(
        r###"struct A{x:i32}struct B{x:i32}trait Take{fn take(self)->i32;}impl Take for A{fn take(self)->i32{self.x}}impl Take for B{fn take(self)->i32{self.x}}fn run()->i32{A{x:3}.take()+B{x:4}.take()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_distinct_traits_inline_multi_bounds() {
    runs(
        r###"struct S{x:i32}trait A{fn a(self)->i32;}trait B{fn b(self)->i32;}impl A for S{fn a(self)->i32{self.x}}impl B for S{fn b(self)->i32{self.x}}fn both<T:A+B>(x:T,y:T)->i32{x.a()+y.b()}fn run()->i32{both::<S>(S{x:3},S{x:4})}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_generic_nominal_trait_impl() {
    runs(
        r###"struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T> Value for W<T>{fn value(self)->i32{7}}fn run()->i32{W::<u8>{x:1u8}.value()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_generic_impl_signature_substitution() {
    runs(
        r###"struct W<T>{x:T}trait Replace{fn replace(self,other:Self)->Self;}impl<T> Replace for W<T>{fn replace(self,other:Self)->Self{other}}fn run()->i32{let w=W::<i32>{x:3}.replace(W::<i32>{x:7});w.x}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_universal_bound_t_path_and_unique_scoped_dot() {
    runs(
        r###"struct S{x:i32}trait Value{fn make(x:i32)->Self;fn value(self)->i32;}impl Value for S{fn make(x:i32)->Self{S{x}}fn value(self)->i32{self.x}}fn call<T:Value>(x:i32)->i32{T::make(x).value()}fn run()->i32{call::<S>(7)}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_rigid_bound_forwarding_direct_generic_call() {
    runs(
        r###"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}fn bounded<T:Value>(x:T)->i32{x.value()}fn forward<U:Value>(x:U)->i32{bounded::<U>(x)}fn run()->i32{forward::<S>(S{x:7})}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_conditional_impl_prerequisite_proof() {
    runs(
        r###"struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn run()->i32{W::<S>{x:S{x:7}}.value()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_nested_generic_nominal_conditional_dispatch() {
    runs(
        r###"struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn consume<T:Value>(x:T)->i32{<T as Value>::value(x)}fn run()->i32{consume::<W<W<S>>>(W::<W<S>>{x:W::<S>{x:S{x:7}}})}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_receiver_consumed_once() {
    runs(
        r###"struct S{x:i32}struct Pair{a:S,b:S}trait Take{fn take(self)->i32;}impl Take for Pair{fn take(self)->i32{let a=self.a;let b=self.b;a.x+b.x}}fn run()->i32{Pair{a:S{x:3},b:S{x:4}}.take()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_receiver_before_arguments_once_ltr() {
    runs(
        r###"struct S{x:i32}trait Combine{fn combine(self,a:i32,b:i32)->i32;}impl Combine for S{fn combine(self,a:i32,b:i32)->i32{self.x+a+b}}fn receiver(n:&mut i32)->S{*n=*n*10+1;S{x:1}}fn arg(n:&mut i32,d:i32)->i32{*n=*n*10+d;d}fn run()->i32{let mut n=0;let x=receiver(&mut n).combine(arg(&mut n,2),arg(&mut n,3));if n==123 && x==6{7}else{0}}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_explicit_ufcs_dispatch() {
    runs(
        r###"struct S{x:i32}trait A{fn value(self)->i32;}trait B{fn value(self)->i32;}impl A for S{fn value(self)->i32{self.x}}impl B for S{fn value(self)->i32{self.x+1}}fn run()->i32{<S as A>::value(S{x:3})+<S as B>::value(S{x:3})}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_public_trait_and_impl_module_visibility() {
    runs(
        r###"mod m{pub struct S{pub x:i32}pub trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}}use m::Value;fn run()->i32{m::S{x:7}.value()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_valid_unused_bound_function_and_impl() {
    runs(
        r###"struct S{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn unused<T:Value>(x:T)->i32{<T as Value>::value(x)}fn run()->i32{7}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn rejects_invalid_unused_impl_missing_required_item() {
    rejects(
        "negative: unused_impl_missing_required_item",
        r###"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_impl_extra_item() {
    rejects(
        "negative: unused_impl_extra_item",
        r###"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{self.x}fn extra(self)->i32{self.x}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_impl_parameter_mismatch() {
    rejects(
        "negative: unused_impl_parameter_mismatch",
        r###"struct S{x:i32}trait Value{fn value(self,x:i32)->i32;}impl Value for S{fn value(self,x:u8)->i32{self.x}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_impl_return_mismatch() {
    rejects(
        "negative: unused_impl_return_mismatch",
        r###"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->u8{7u8}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_impl_receiver_kind_mismatch() {
    rejects(
        "negative: unused_impl_receiver_kind_mismatch",
        r###"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value()->i32{7}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_duplicate_required_trait_item() {
    rejects(
        "negative: duplicate_required_trait_item",
        r###"trait Value{fn value(self)->i32;fn value(self)->i32;}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_duplicate_same_trait_same_type_impl() {
    rejects(
        "negative: duplicate_same_trait_same_type_impl",
        r###"struct S{x:i32}trait Value{fn value(self)->i32;}impl Value for S{fn value(self)->i32{3}}impl Value for S{fn value(self)->i32{4}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_overlapping_generic_concrete_impl() {
    rejects(
        "negative: overlapping_generic_concrete_impl",
        r###"struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T> Value for W<T>{fn value(self)->i32{3}}impl Value for W<i32>{fn value(self)->i32{4}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_impl_unconstrained_parameter() {
    rejects(
        "negative: unused_impl_unconstrained_parameter",
        r###"struct S{x:i32}trait Value{fn value(self)->i32;}impl<T> Value for S{fn value(self)->i32{self.x}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_header_unknown_trait() {
    rejects(
        "negative: unused_header_unknown_trait",
        r###"fn unused<T:Missing>(x:T)->T{x}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_universal_body_duplicate_move() {
    rejects(
        "negative: universal_body_duplicate_move",
        r###"trait Value{fn value(self)->i32;}fn bad<T:Value>(x:T)->(T,T){(x,x)}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_missing_bound_direct_call_and_conditional_prerequisite() {
    rejects(
        "negative: missing_bound_direct_call_and_conditional_prerequisite",
        r###"struct U{x:i32}struct W<T>{x:T}trait Value{fn value(self)->i32;}impl<T:Value> Value for W<T>{fn value(self)->i32{self.x.value()}}fn bounded<T:Value>(x:T)->i32{x.value()}fn missing<T>(x:T)->i32{bounded::<T>(x)}fn dead(){if false{bounded::<U>(U{x:7});}}fn run()->i32{W::<U>{x:U{x:7}}.value()}
fn main(){run();}
"###,
    );
}

#[test]
fn accepts_historical_default_method_body() {
    runs(
        r###"struct S{x:i32}trait Value{fn value()->i32{7}}impl Value for S{}fn run()->i32{<S as Value>::value()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn accepts_generic_trait_parameters_with_unique_application() {
    runs(
        r###"struct S{x:i32}trait Value<T>{fn value(self,x:T)->i32;}impl Value<i32> for S{fn value(self,x:i32)->i32{self.x+x}}fn run()->i32{S{x:3}.value(4)}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn runs_generic_trait_method() {
    // Formerly a legal-unsupported boundary; stock rustc 2024 prints
    // "traits oracle ok" for this exact source (checked 2026-10-24).
    runs(
        r###"struct S{x:i32}trait Choose{fn choose<T>(self,x:T)->T;}impl Choose for S{fn choose<T>(self,x:T)->T{x}}fn run()->i32{let b=S{x:1}.choose(true);if b{S{x:0}.choose::<i32>(7)}else{0}}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}

#[test]
fn runs_borrowed_receiver_and_copy_bound() {
    runs(
        r###"struct S{x:i32}trait Read{fn read(&self)->i32;}impl Read for S{fn read(&self)->i32{self.x}}fn dup<T:Copy>(x:T)->(T,T){(x,x)}fn run()->i32{let s=S{x:3};let p=dup::<i32>(2);s.read()+p.0+p.1}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        "traits oracle ok\n",
    );
}
