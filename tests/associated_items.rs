//! Frozen exact32 inherent associated-items acceptance corpus.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[path = "support/iterator_oracle.rs"]
mod disjoint_oracle;

#[test]
fn disjoint_inherent_receiver_arguments_preserve_native_selection_and_order() {
    disjoint_oracle::stock_native(
        r#"
struct Token<T> { value: T }
impl Token<u8> { fn get(self) -> u8 { self.value } }
impl Token<u64> {
    fn get(self) -> u64 { self.value }
    fn with(self, extra: u64) -> u64 {
        println!("callee");
        self.value + extra
    }
}
fn receiver() -> Token<u64> {
    println!("receiver");
    Token::<u64> { value: 40u64 }
}
fn argument() -> u64 { println!("argument"); 2u64 }
fn main() {
    println!("{}", (Token::<u8> { value: 255u8 }).get());
    println!("{}", (Token::<u64> { value: 18446744073709551615u64 }).get());
    println!("{}", receiver().with(argument()));
}
"#,
    );
}

#[test]
fn disjoint_inherent_receiver_filter_keeps_rigid_generic_arguments() {
    disjoint_oracle::stock_native(
        r#"
struct Token<T> { value: T }
impl<T> Token<(T, u8)> { fn get(self) -> T { self.value.0 } }
impl<T> Token<(T, u64)> { fn get(self) -> T { self.value.0 } }
fn first<T>(value: T) -> T { Token::<(T, u8)> { value: (value, 1u8) }.get() }
fn second<T>(value: T) -> T { Token::<(T, u64)> { value: (value, 2u64) }.get() }
fn main() { println!("{}", first(19i32) + second(23i32)); }
"#,
    );
}

#[test]
fn disjoint_inherent_receiver_filter_cannot_select_wrong_or_unknown_arguments() {
    for source in [
        "struct Token<T>{value:T}impl Token<u8>{fn get(self)->u8{self.value}}impl Token<u64>{fn get(self)->u64{self.value}}fn main(){let x=Token::<bool>{value:true}.get();}",
        "struct Token<T>{value:T}impl Token<u8>{fn get(self)->u8{self.value}}impl Token<u64>{fn get(self)->u64{self.value}}fn bad<T>(value:T){Token::<T>{value:value}.get();}fn main(){}",
    ] {
        disjoint_oracle::stock_and_native_reject(source);
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
            let path = base.join(format!(
                "jr-associated-items-{}-{epoch}-{id}",
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
fn accepts_associated_function_struct() {
    runs(
        r###"struct S{x:i32}impl S{fn make(x:i32)->S{S{x}}}fn run()->i32{S::make(7).x}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_associated_function_enum() {
    runs(
        r###"enum E{A(i32),B}impl E{fn make(x:i32)->E{E::A(x)}}fn run()->i32{match E::make(7){E::A(x)=>x,E::B=>0}}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_consuming_self_dot_and_qualified_equivalence() {
    runs(
        r###"struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn run()->i32{S{x:3}.take()+S::take(S{x:4})}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_mut_self_local_mutation() {
    runs(
        r###"struct S{x:i32}impl S{fn add(mut self,n:i32)->i32{self.x=self.x+n;self.x}}fn run()->i32{let s=S{x:3};s.add(4)}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_impl_binder_receiver_matching() {
    runs(
        r###"struct W<T>{value:T}impl<T> W<T>{fn take(self)->T{self.value}fn make(value:T)->W<T>{W::<T>{value}}}fn run()->i32{W::<i32>::make(7).take()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_method_binder_explicit_arguments() {
    runs(
        r###"struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}fn identity<U>(u:U)->U{u}}fn run()->i32{let a=S{x:0}.choose::<i32>(3);let b=S::identity::<i32>(4);a+b}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_impl_method_distinct_binders_forwarding() {
    runs(
        r###"struct Pair<T,U>{a:T,b:U}struct W<T>{value:T}impl<T> W<T>{fn pair<U>(self,u:U)->Pair<U,T>{Pair::<U,T>{a:u,b:self.value}}}fn forward<T,U>(a:T,b:U)->Pair<U,T>{W::<T>{value:a}.pair::<U>(b)}fn run()->i32{let p=forward::<i32,u8>(3,4u8);p.a as i32+p.b}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_unrelated_same_spelled_binder_identity() {
    runs(
        r###"struct A<T>{x:T}struct B<T>{x:T}impl<T> A<T>{fn take(self)->T{self.x}}impl<T> B<T>{fn take(self)->T{self.x}}fn run()->i32{let a=A::<u8>{x:3u8}.take();let b=B::<i32>{x:4}.take();a as i32+b}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_self_type_constructor_and_associated_path() {
    runs(
        r###"struct S{x:i32}impl S{fn make(x:i32)->Self{Self{x}}fn via(x:i32)->Self{Self::make(x)}fn take(self)->i32{self.x}}enum E{Item(i32)}impl E{fn make(x:i32)->Self{Self::Item(x)}fn take(self)->i32{match self{Self::Item(x)=>x}}}fn run()->i32{S::via(3).take()+E::make(4).take()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_disjoint_concrete_impl_same_method() {
    runs(
        r###"struct W<T>{x:T}impl W<u8>{fn score(self)->i32{1}}impl W<u16>{fn score(self)->i32{2}}struct Pair<T,U>{a:T,b:U}impl<T> Pair<T,T>{fn score(self)->i32{3}}impl Pair<i32,u8>{fn score(self)->i32{1}}fn run()->i32{W::<u8>{x:0u8}.score()+W::<u16>{x:0u16}.score()+Pair::<i32,i32>{a:0,b:0}.score()+Pair::<i32,u8>{a:0,b:0u8}.score()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_separate_impl_disjoint_item_names() {
    runs(
        r###"struct W<T>{x:T}impl<T> W<T>{fn take(self)->T{self.x}}impl W<i32>{fn make(x:i32)->W<i32>{W::<i32>{x}}}fn run()->i32{W::<i32>::make(7).take()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_nominal_owner_identity_same_item_name() {
    runs(
        r###"mod a{pub struct S{pub x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}mod b{pub struct S{pub x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}fn run()->i32{a::S::make(3).take()+b::S::make(4).take()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_receiver_then_arguments_once_ltr() {
    runs(
        r###"struct S{x:i32}impl S{fn combine(self,a:i32,b:i32)->i32{self.x+a+b}}fn receiver(n:&mut i32)->S{*n=*n*10+1;S{x:1}}fn arg(n:&mut i32,d:i32)->i32{*n=*n*10+d;d}fn run()->i32{let mut n=0;let x=receiver(&mut n).combine(arg(&mut n,2),arg(&mut n,3));if n==123 && x==6{7}else{0}}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_noncopy_receiver_partial_field_moves() {
    runs(
        r###"struct S{x:i32}struct P{a:S,b:S}impl P{fn take(self)->i32{let a=self.a;let b=self.b;a.x+b.x}}fn run()->i32{P{a:S{x:3},b:S{x:4}}.take()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_rigid_generic_method_forwarding() {
    runs(
        r###"struct W<T>{x:T}impl<T> W<T>{fn pair<U>(self,u:U)->(T,U){(self.x,u)}fn forward<U>(self,u:U)->(T,U){self.pair::<U>(u)}}fn bridge<T,U>(x:T,u:U)->(T,U){W::<T>{x}.forward::<U>(u)}fn run()->i32{let p=bridge::<u8,i32>(3u8,4);p.0 as i32+p.1}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_exact_instance_method_recursion() {
    runs(
        r###"struct W<T>{x:T}impl<T> W<T>{fn recur<U>(self,u:U,n:i32)->(T,U){if n==0{(self.x,u)}else{self.recur::<U>(u,n-1)}}}fn run()->i32{let p=W::<i32>{x:3}.recur::<u8>(4u8,3);p.0+p.1 as i32}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_unused_method_type_parameter_explicit() {
    runs(
        r###"struct S{x:i32}impl S{fn take<U>(self)->i32{self.x}fn never_used<V>(x:i32)->i32{x}}fn run()->i32{S{x:7}.take::<u64>()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_visible_public_method_across_module() {
    runs(
        r###"mod a{pub struct S{x:i32}impl S{pub fn make(x:i32)->S{S{x}}pub fn take(self)->i32{self.x}}}mod b{impl super::a::S{pub fn consume(self)->i32{self.take()}}}fn run()->i32{a::S::make(7).consume()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn rejects_invalid_use_after_consuming_receiver() {
    rejects(
        "negative: use_after_consuming_receiver",
        r###"struct S{x:i32}impl S{fn take(self)->i32{self.x}}fn run()->i32{let s=S{x:7};let n=s.take();s.x}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_generic_method_duplicate_move() {
    rejects(
        "negative: unused_generic_method_duplicate_move",
        r###"struct W<T>{x:T}impl<T> W<T>{fn bad<U>(self,u:U)->(U,U){(u,u)}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unused_method_type_error() {
    rejects(
        "negative: unused_method_type_error",
        r###"struct S{x:i32}impl S{fn bad<U>(self)->U{7}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_unconstrained_impl_parameter_header() {
    rejects(
        "negative: unconstrained_impl_parameter_header",
        r###"struct S{x:i32}impl<T> S{fn unused(self)->i32{self.x}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_duplicate_visible_impl_method_parameter_name() {
    rejects(
        "negative: duplicate_visible_impl_method_parameter_name",
        r###"struct W<T>{x:T}impl<T> W<T>{fn bad<T>(self,x:T)->T{x}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_duplicate_items_within_impl() {
    rejects(
        "negative: duplicate_items_within_impl",
        r###"struct S{x:i32}impl S{fn clash(self)->i32{3}fn clash(self)->i32{4}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_overlapping_generic_concrete_same_named_item() {
    rejects(
        "negative: overlapping_generic_concrete_same_named_item",
        r###"struct W<T>{x:T}impl<T> W<T>{fn clash(self)->i32{3}}impl W<i32>{fn clash(self)->i32{4}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_private_method_access() {
    rejects(
        "negative: private_method_access",
        r###"mod a{pub struct S{pub x:i32}impl S{fn take(self)->i32{self.x}}}fn run()->i32{a::S{x:7}.take()}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_impl_site_cannot_access_definition_private_field() {
    rejects(
        "negative: impl_site_cannot_access_definition_private_field",
        r###"mod a{pub struct S{x:i32}}mod b{impl super::a::S{pub fn leak(self)->i32{self.x}}}fn run()->i32{7}
fn main(){run();}
"###,
    );
}

#[test]
fn rejects_invalid_associated_and_receiver_call_arity_misuse() {
    rejects(
        "negative: associated_and_receiver_call_arity_misuse",
        r###"struct S{x:i32}impl S{fn make(x:i32)->S{S{x}}fn take(self)->i32{self.x}}fn run()->i32{let s=S{x:7};s.make(1);S::take();S::make(1,2);7}
fn main(){run();}
"###,
    );
}

#[test]
fn accepts_trait_dispatch() {
    runs(
        r###"struct S{x:i32}trait Take{fn take(self)->i32;}impl Take for S{fn take(self)->i32{self.x}}fn run()->i32{S{x:7}.take()}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_borrowed_receiver_autoref_autoderef() {
    runs(
        r###"struct S{x:i32}impl S{fn get(&self)->i32{self.x}}fn run()->i32{let s=S{x:3};let a=s.get();let r=&s;a+r.get()+1}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_omitted_method_type_arguments() {
    runs(
        r###"struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn run()->i32{S{x:0}.choose(7)}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}

#[test]
fn accepts_partial_method_type_hole() {
    runs(
        r###"struct S{x:i32}impl S{fn choose<U>(self,u:U)->U{u}}fn run()->i32{S{x:0}.choose::<_>(7)}
fn main(){if run()==7{println!("associated items oracle ok");}}
"###,
        "associated items oracle ok\n",
    );
}
