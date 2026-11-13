// E1 free-function lifetime elision, frozen stock-correct twenty-case corpus.
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
                "jr-lifetime-elision-{}-{epoch}-{id}",
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
fn accepts_shared_identity() {
    runs(
        r#"fn id(x:&i32)->&i32{x} fn run()->i32{let x=7;*id(&x)}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_mutable_identity() {
    runs(
        r#"fn id(x:&mut i32)->&mut i32{x} fn run()->i32{let mut x=3;let r=id(&mut x);*r=7;x}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_shared_projected() {
    runs(
        r#"struct S{x:i32} fn get(p:&S)->&i32{&(*p).x} fn run()->i32{let p=S{x:7};*get(&p)}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_mutable_projected() {
    runs(
        r#"fn get(p:&mut(i32,i32))->&mut i32{&mut (*p).1} fn run()->i32{let mut p=(3,4);let r=get(&mut p);*r=7;p.1}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_shared_forward_recursive() {
    runs(
        r#"fn recur(x:&i32,n:i32)->&i32{if n==0{x}else{recur(x,n-1)}} fn forward(x:&i32)->&i32{recur(x,3)} fn run()->i32{let x=7;*forward(&x)}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_mutable_forward_recursive() {
    runs(
        r#"fn recur(x:&mut i32,n:i32)->&mut i32{if n==0{x}else{recur(x,n-1)}} fn forward(x:&mut i32)->&mut i32{recur(x,3)} fn run()->i32{let mut x=3;let r=forward(&mut x);*r=7;x}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn rejects_invalid_repeated_named_shared_region() {
    // Stock rejects this fixture with E0106.
    rejects(
        "repeated_named_shared_region",
        r#"fn choose<'a>(x:&'a i32,y:&'a i32,b:bool)->&i32{if b{x}else{y}} fn run()->i32{let x=3;let y=7;*choose(&x,&y,false)}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_repeated_named_mutable_region() {
    // Stock rejects this fixture with E0106.
    rejects(
        "repeated_named_mutable_region",
        r#"fn choose<'a>(x:&'a mut i32,y:&'a mut i32,b:bool)->&mut i32{if b{x}else{y}} fn run()->i32{let mut x=3;let mut y=4;let r=choose(&mut x,&mut y,false);*r=7;y}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_scalar_arguments_ignored() {
    runs(
        r#"fn id(n:i32,x:&i32,b:bool,k:usize)->&i32{x} fn run()->i32{let x=7;*id(0,&x,true,1usize)}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_explicit_output_independent_fresh_input() {
    runs(
        r#"fn pick<'a>(x:&'a i32,y:&bool)->&'a i32{x} fn run()->i32{let x=7;let r:&i32;{let y=true;r=pick(&x,&y);}*r}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_last_use_then_owner_mutation() {
    runs(
        r#"fn id(x:&i32)->&i32{x} fn run()->i32{let mut x=3;let r=id(&x);let n=*r;x=7;n+x}
fn main() { if run() == 10 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_carrier_storage_dead_owner_live() {
    runs(
        r#"fn id(x:&mut i32)->&mut i32{x} fn run()->i32{let mut x=3;let r:&mut i32;{let p=&mut x;r=id(p);}*r=7;x}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn rejects_invalid_zero_input_output_lifetime() {
    // Stock rejects this fixture with E0106.
    rejects(
        "zero_input_output_lifetime",
        r#"fn bad()->&i32{loop{}} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_two_anonymous_input_ambiguity() {
    // Stock rejects this fixture with E0106.
    rejects(
        "two_anonymous_input_ambiguity",
        r#"fn bad(x:&i32,y:&i32)->&i32{x} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_explicit_and_fresh_output_ambiguity() {
    // Stock rejects this fixture with E0106.
    rejects(
        "explicit_and_fresh_output_ambiguity",
        r#"fn bad<'a>(x:&'a i32,y:&i32)->&i32{x} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_fresh_input_cannot_satisfy_named_output() {
    // Stock rejects this fixture with E0621.
    rejects(
        "fresh_input_cannot_satisfy_named_output",
        r#"fn bad<'a>(x:&'a i32,y:&i32)->&'a i32{y} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_local_owner_return() {
    // Stock rejects this fixture with E0515.
    rejects(
        "local_owner_return",
        r#"fn bad(x:&i32)->&i32{let y=7;&y} fn run()->i32{7}
fn main() { run(); }
"#,
    );
}

#[test]
fn rejects_invalid_same_region_dependency_storage_pin() {
    // Stock rejects this fixture with E0597.
    rejects(
        "same_region_dependency_storage_pin",
        r#"fn pick<'a>(x:&'a bool,y:&'a i32)->&'a i32{y} fn run()->i32{let y=7;let r:&i32;{let x=true;r=pick(&x,&y);}*r}
fn main() { run(); }
"#,
    );
}

#[test]
fn accepts_implicit_autoderef_projected() {
    runs(
        r#"struct S{x:i32} fn get(p:&S)->&i32{&p.x} fn run()->i32{let p=S{x:7};*get(&p)}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_mixed_kind_inputs() {
    runs(
        r#"fn pick<'a>(x:&'a mut i32,y:&'a bool)->&'a mut i32{x} fn run()->i32{let mut x=3;let y=true;let r=pick(&mut x,&y);*r=7;x}
fn main() { if run() == 7 { println!("elision oracle ok"); } }
"#,
        "elision oracle ok\n",
    );
}

#[test]
fn accepts_static_type_and_callable_bounds() {
    runs(
        r#"trait Show { fn show(&self) -> i32; }
impl Show for i32 { fn show(&self) -> i32 { *self } }
fn bounded<T: Show + 'static>(value: T) -> i32 { value.show() }
fn via_where<T>(value: T) -> i32 where T: Show + 'static { bounded(value) }
fn invoke(f: impl Fn(i32) -> i32 + 'static) -> i32 { f(2) }
fn accept<T: 'static>(value: T) {}
fn forward(value: &'static str) { accept(value); }
fn main() { let value = 5; forward("hello"); println!("{} {}", via_where(7), invoke(move |x| x + value)); }
"#,
        "7 7\n",
    );
}

#[test]
fn accepts_opaque_outlives_bounds() {
    runs(
        r#"fn named<'a>(value: &'a i32) -> impl Fn() -> i32 + 'a { move || *value }
fn inferred(value: &i32) -> impl Fn() -> i32 + '_ { move || *value }
fn owned(value: i32) -> impl Fn() -> i32 + 'static { move || value }
fn main() { let value = 7; let a = named(&value); let b = inferred(&value); let c = owned(7); println!("{} {} {}", a(), b(), c()); }
"#,
        "7 7 7\n",
    );
    runs(
        r#"fn pair<'a>(values: (&'a i32, &'a i32)) -> impl Fn() -> i32 + 'a {
    move || *values.0 + *values.1
}
fn main() { let a = 3; let b = 4; let f = pair((&a, &b)); println!("{}", f()); }
"#,
        "7\n",
    );
}

#[test]
fn accepts_named_type_outlives_bounds() {
    runs(
        r#"use std::fmt::Display;
fn tie<'a, T: Display + 'a>(label: &'a str, value: T) -> &'a str { println!("{} {}", label, value); label }
fn longest<'a, T>(items: &'a [T], fallback: &'a T) -> &'a T where T: PartialOrd + 'a {
    let mut best = fallback;
    for item in items { if item > best { best = item; } }
    best
}
fn store<'a, T: 'a>(slot: &mut Vec<T>, t: T) { slot.push(t); }
fn main() {
    let name = String::from("count");
    let n = 4;
    let r = tie(name.as_str(), &n);
    let words = vec![String::from("b"), String::from("zz")];
    let zero = String::from("a");
    let s = String::from("q"); let mut v: Vec<&str> = Vec::new(); store(&mut v, s.as_str());
    println!("{} {} {:?}", r, longest(&words, &zero), v);
}
"#,
        "count 4\ncount zz [\"q\"]\n",
    );
}

#[test]
fn rejects_named_type_outlives_violations() {
    rejects(
        "struct result tied to a shorter type argument",
        r#"struct W<'a> { s: &'a () }
fn wrap<'a, T: 'a>(s: &'a (), _t: T) -> W<'a> { W { s } }
fn make<'a, 'b>(s: &'a (), t: &'b u8) -> W<'a> { wrap(s, t) }
fn main() {}
"#,
    );
    rejects(
        "reference result tied to a shorter type argument",
        r#"fn wrap<'a, T: 'a>(s: &'a (), _t: T) -> &'a () { s }
fn make<'a, 'b>(s: &'a (), t: &'b u8) -> &'a () { wrap(s, t) }
fn main() {}
"#,
    );
    rejects(
        "type argument borrow ends before the bounded result",
        r#"fn tie<'a, T: 'a>(s: &'a i32, _t: T) -> &'a i32 { s }
fn main() { let a = 1; let r; { let b = 2; r = tie(&a, &b); } println!("{}", r); }
"#,
    );
    rejects(
        "undeclared lifetime in a type outlives bound",
        r#"fn tie<T: 'x>(t: T) -> T { t }
fn main() {}
"#,
    );
}

#[test]
fn rejects_static_type_bound_on_borrowed_actual() {
    rejects(
        "static generic actual",
        r#"fn bounded<T: 'static>(value: T) {}
fn main() { let value = 7; bounded(&value); }
"#,
    );
    rejects(
        "static generic forwarding checked even when unused",
        r#"fn bounded<T: 'static>(value: T) {}
fn invalid<T>(value: T) { bounded(value); }
fn main() {}
"#,
    );
    rejects(
        "static where bound",
        r#"fn bounded<T>(value: T) where T: 'static {}
fn main() { let value = 7; bounded(&value); }
"#,
    );
    rejects(
        "static obligation without a runtime argument",
        r#"fn bounded<T: 'static>() {}
fn invalid<T>() { bounded::<T>(); }
fn main() {}
"#,
    );
    rejects(
        "phantom lifetime must not be mistaken for static",
        r#"use std::marker::PhantomData;
fn bounded<T: 'static>(value: T) {}
fn invalid<'a>(value: PhantomData<&'a i32>) { bounded(value); }
fn main() {}
"#,
    );
    rejects(
        "raw pointee lifetime must not be mistaken for static",
        r#"fn bounded<T: 'static>(value: T) {}
fn invalid<'a>(value: *const &'a i32) { bounded(value); }
fn main() {}
"#,
    );
    rejects(
        "function item type actual lifetime must not be mistaken for static",
        r#"fn bounded<T: 'static>(value: T) {}
fn id<T>(value: T) -> T { value }
fn invalid<'a>() { bounded(id::<&'a i32>); }
fn main() {}
"#,
    );
    rejects(
        "trait implementation may not add a static requirement",
        r#"trait Consume { fn consume<T>(value: T); }
struct Owner;
impl Consume for Owner { fn consume<T: 'static>(value: T) {} }
fn main() {}
"#,
    );
}

#[test]
fn rejects_static_callable_bound_on_borrowed_capture() {
    rejects(
        "static closure with borrowed capture",
        r#"fn invoke(f: impl Fn() -> i32 + 'static) -> i32 { f() }
fn main() { let value = 7; let reference = &value; println!("{}", invoke(move || *reference)); }
"#,
    );
}

#[test]
fn rejects_opaque_outlives_violation_even_when_unused() {
    rejects(
        "opaque named capture cannot escape at its caller",
        r#"fn capture<'a>(value: &'a i32) -> impl Fn() -> i32 + 'a { move || *value }
fn main() { let f; { let value = 7; f = capture(&value); } println!("{}", f()); }
"#,
    );
    rejects(
        "opaque inferred lifetime remains ambiguous with repeated named inputs",
        r#"fn invalid<'a>(a: &'a i32, b: &'a i32) -> impl Fn() -> i32 + '_ { move || *a + *b }
fn main() {}
"#,
    );
    rejects(
        "opaque static capture",
        r#"fn invalid(value: &i32) -> impl Fn() -> i32 + 'static { move || *value }
fn main() {}
"#,
    );
    rejects(
        "opaque wrong named lifetime",
        r#"fn invalid<'a, 'b>(a: &'a i32, b: &'b i32) -> impl Fn() -> i32 + 'a { move || *b }
fn main() {}
"#,
    );
    rejects(
        "opaque phantom static lifetime",
        r#"use std::marker::PhantomData;
fn invalid<'a>(value: PhantomData<&'a i32>) -> impl Sized + 'static { value }
fn main() {}
"#,
    );
}

#[test]
fn accepts_static_owned_trait_object() {
    runs(
        r#"trait Show { fn show(&self) -> i32; }
impl Show for i32 { fn show(&self) -> i32 { *self } }
fn main() { let value: Box<dyn Show + 'static> = Box::new(7); println!("{}", value.show()); }
"#,
        "7\n",
    );
}

#[test]
fn accepts_physical_borrowed_opaque_transport() {
    runs(
        r#"fn capture<'a>(value: &'a i32) -> impl Fn() -> i32 + 'a { move || *value }
fn forward<'a>(value: &'a i32) -> impl Fn() -> i32 + 'a { capture(value) }
fn main() { let value = 7; let f = forward(&value); println!("{} {}", f(), f()); }
"#,
        "7 7\n",
    );
    runs(
        r#"fn capture<'a>(value: &'a mut i32) -> impl FnMut() -> i32 + 'a {
    move || { *value += 1; *value }
}
fn main() { let mut value = 5; { let mut f = capture(&mut value); println!("{} {}", f(), f()); } println!("{}", value); }
"#,
        "6 7\n7\n",
    );
    runs(
        r#"fn capture<'a>(value: &'a &'a i32) -> impl Fn() -> i32 + 'a { move || **value }
fn main() { let value = 7; let reference = &value; let f = capture(&reference); println!("{}", f()); }
"#,
        "7\n",
    );
}

#[test]
fn rejects_physical_opaque_alias_and_nested_escape() {
    rejects(
        "physical nested reference retains its inner borrow",
        r#"fn capture<'a>(value: &'a &'a i32) -> impl Fn() -> i32 + 'a { move || **value }
fn main() { let reference; let f; { let value = 7; reference = &value; f = capture(&reference); } println!("{}", f()); }
"#,
    );
    rejects(
        "physical mutable capture retains exclusive borrow",
        r#"fn capture<'a>(value: &'a mut i32) -> impl FnMut() -> i32 + 'a { move || { *value += 1; *value } }
fn main() { let mut value = 5; let mut f = capture(&mut value); value = 9; println!("{}", f()); }
"#,
    );
    rejects(
        "physical nested capture retains all input origins",
        r#"fn capture<'a>(values: (&'a i32, &'a i32)) -> impl Fn() -> i32 + 'a { move || *values.0 + *values.1 }
fn main() { let value = 3; let f; { let short = 4; f = capture((&value, &short)); } println!("{}", f()); }
"#,
    );
    rejects(
        "physical forwarded opaque retains defining lifetime",
        r#"fn capture<'a>(value: &'a i32) -> impl Fn() -> i32 + 'a { move || *value }
fn forward<'a>(value: &'a i32) -> impl Fn() -> i32 + 'a { capture(value) }
fn main() { let f; { let value = 7; f = forward(&value); } println!("{}", f()); }
"#,
    );
}

#[test]
fn rejects_erased_object_borrow_escaping_its_storage() {
    rejects(
        "owning static object must not erase borrowed payload",
        r#"trait Show { fn show(&self) -> i32; }
struct Borrowed<'a>(&'a i32);
impl<'a> Show for Borrowed<'a> { fn show(&self) -> i32 { *self.0 } }
fn main() { let object: Box<dyn Show + 'static>; { let value = 7; object = Box::new(Borrowed(&value)); } println!("{}", object.show()); }
"#,
    );
}
