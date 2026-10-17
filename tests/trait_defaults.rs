//! Executable source-trait-default conformance against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static NEXT: AtomicU64 = AtomicU64::new(0);
struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for _ in 0..100 {
            let path = base.join(format!(
                "jr-defaults-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("scratch: {e}"),
            }
        }
        panic!("scratch allocation exhausted")
    }
    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn file(path: &Path) -> fs::File {
    OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap()
}
fn capture(command: &mut Command, scratch: &Scratch) -> (i32, Vec<u8>, Vec<u8>) {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let out = scratch.path(&format!("stdout-{id}"));
    let err = scratch.path(&format!("stderr-{id}"));
    let mut child = Running(
        command
            .stdin(Stdio::null())
            .stdout(file(&out))
            .stderr(file(&err))
            .spawn()
            .unwrap(),
    );
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "timeout: {command:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    for path in [&out, &err] {
        assert!(
            fs::metadata(path).unwrap().len() <= 65536,
            "capture exceeded 64KiB"
        );
    }
    (
        status.code().expect("process terminated by signal"),
        fs::read(out).unwrap(),
        fs::read(err).unwrap(),
    )
}
fn acceptance(source: &str, expected: Option<&str>) {
    let scratch = Scratch::new();
    let input = scratch.path("input.rs");
    file(&input).write_all(source.as_bytes()).unwrap();
    let stock = scratch.path("stock");
    let result = capture(
        Command::new("rustc")
            .arg("--edition=2021")
            .arg(&input)
            .arg("-o")
            .arg(&stock),
        &scratch,
    );
    if let Some(stdout) = expected {
        assert_eq!(
            result.0,
            0,
            "stock rejected: {}\n{source}",
            String::from_utf8_lossy(&result.2)
        );
        let run = capture(&mut Command::new(&stock), &scratch);
        assert_eq!(
            run,
            (0, stdout.as_bytes().to_vec(), Vec::new()),
            "stock runtime: {source}"
        );
    } else {
        assert_eq!(result.0, 1, "stock accepted invalid fixture: {source}");
        assert!(!result.2.is_empty());
        assert!(!stock.exists(), "stock published invalid executable");
    }
    for workers in [1, 8] {
        for check in [true, false] {
            let output = scratch.path(&format!("native-{workers}-{check}"));
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(&input)
                .args(["--workers", &workers.to_string()]);
            if check {
                command.arg("--check");
            } else {
                command.arg("-o").arg(&output);
            }
            let result = capture(&mut command, &scratch);
            assert_eq!(
                result.0,
                if expected.is_some() { 0 } else { 1 },
                "workers={workers} check={check}: {}\n{source}",
                String::from_utf8_lossy(&result.2)
            );
            assert!(
                !String::from_utf8_lossy(&result.2).contains("panicked at"),
                "compiler panic"
            );
            if check || expected.is_none() {
                assert!(!output.exists(), "CHECK/failure published executable");
                if expected.is_none() {
                    assert!(!result.2.is_empty(), "missing diagnostic");
                    let diagnostic = String::from_utf8_lossy(&result.2);
                    assert!(
                        !diagnostic.contains("source trait default method bodies are unsupported")
                            && !diagnostic.contains(
                                "trait-owned callable is not a registered source default provider"
                            ),
                        "invalid body was fenced off instead of validated: {diagnostic}"
                    );
                }
            } else if let Some(expected_stdout) = expected {
                let bytes = fs::read(&output).unwrap();
                assert!(bytes.len() >= 64);
                assert_eq!(&bytes[..6], b"\x7fELF\x02\x01");
                let run = capture(&mut Command::new(&output), &scratch);
                assert_eq!(
                    run,
                    (0, expected_stdout.as_bytes().to_vec(), Vec::new()),
                    "workers={workers}: {source}"
                );
            }
        }
    }
}
fn valid(source: &str) {
    acceptance(source, Some("defaults ok\n"));
}
fn invalid(source: &str) {
    acceptance(source, None);
}

#[test]
fn unused_valid_default_without_impl() {
    valid(r#"trait Value:Sized{fn value(self)->i32{7}}fn main(){println!("defaults ok");}"#);
}
#[test]
fn unused_by_value_default_without_sized_is_invalid() {
    invalid("trait Value{fn value(self)->i32{7}}fn main(){}");
}
#[test]
fn overridden_by_value_default_without_sized_is_invalid() {
    invalid(
        "struct S{x:i32}trait Value{fn value(self)->i32{7}}impl Value for S{fn value(self)->i32{self.x}}fn main(){}",
    );
}
#[test]
fn borrowed_defaults_without_sized_are_valid() {
    valid(
        r#"struct S{x:i32}trait Value{fn read(&self)->i32;fn bump(&mut self);fn value(&self)->i32{self.read()+1}fn change(&mut self)->i32{self.bump();self.read()}}impl Value for S{fn read(&self)->i32{self.x}fn bump(&mut self){self.x=self.x+1;}}fn main(){let mut s=S{x:6};if s.value()==7&&s.change()==7&&s.read()==7{println!("defaults ok");}}"#,
    );
}
#[test]
fn omitted_method_selects_default() {
    valid(
        r#"struct S{x:i32}trait Value:Sized{fn value(self)->i32{7}}impl Value for S{}fn main(){if (S{x:1}).value()==7{println!("defaults ok");}}"#,
    );
}
#[test]
fn explicit_override_wins() {
    valid(
        r#"struct S{x:i32}trait Value:Sized{fn value(self)->i32{7}}impl Value for S{fn value(self)->i32{8}}fn main(){if (S{x:1}).value()==8{println!("defaults ok");}}"#,
    );
}
#[test]
fn default_calls_required_method_through_generic_caller() {
    valid(
        r#"struct S{x:i32}trait Value:Sized{fn required(self)->i32;fn value(self)->i32{self.required()+1}}impl Value for S{fn required(self)->i32{self.x}}fn call<T:Value>(x:T)->i32{x.value()}fn main(){if call::<S>(S{x:6})==7{println!("defaults ok");}}"#,
    );
}
#[test]
fn generic_impl_instantiates_self() {
    valid(
        r#"struct W<T>{x:T}trait Value:Sized{fn identity(self)->Self{self}fn value(self)->i32{7}}impl<T> Value for W<T>{}fn main(){let w=W::<i32>{x:3}.identity();if w.x==3&&(W::<u8>{x:1u8}).value()==7{println!("defaults ok");}}"#,
    );
}
#[test]
fn multiple_impls_share_default() {
    valid(
        r#"struct A{x:i32}struct B{x:i32}trait Value:Sized{fn required(self)->i32;fn value(self)->i32{self.required()+1}}impl Value for A{fn required(self)->i32{self.x}}impl Value for B{fn required(self)->i32{self.x*2}}fn main(){if (A{x:2}).value()+(B{x:3}).value()==10{println!("defaults ok");}}"#,
    );
}
#[test]
fn finite_recursive_default() {
    valid(
        r#"struct S{x:i32}trait Sum:Sized{fn sum(self,n:i32)->i32{if n==0{0}else{n+self.sum(n-1)}}}impl Sum for S{}fn main(){if (S{x:0}).sum(4)==10{println!("defaults ok");}}"#,
    );
}
#[test]
fn static_associated_default() {
    valid(
        r#"struct S{x:i32}trait Value:Sized{fn value()->i32{7}}impl Value for S{}fn main(){if <S as Value>::value()==7{println!("defaults ok");}}"#,
    );
}
#[test]
fn historical_default_fixture_stock_and_native() {
    acceptance(
        r###"struct S{x:i32}trait Value{fn value()->i32{7}}impl Value for S{}fn run()->i32{<S as Value>::value()}
fn main(){if run()==7{println!("traits oracle ok");}}
"###,
        Some("traits oracle ok\n"),
    );
}
#[test]
fn borrowed_and_mutably_borrowed_defaults() {
    valid(
        r#"struct S{x:i32}trait Value:Sized{fn read(&self)->i32;fn bump(&mut self);fn value(&self)->i32{self.read()+1}fn change(&mut self)->i32{self.bump();self.read()}}impl Value for S{fn read(&self)->i32{self.x}fn bump(&mut self){self.x=self.x+1;}}fn main(){let mut s=S{x:6};if s.value()==7&&s.change()==7&&s.read()==7{println!("defaults ok");}}"#,
    );
}
#[test]
fn default_only_clone_recipe_discovery() {
    valid(
        r#"struct S{x:i32}trait Value:Sized{fn value(self)->f32{let pair=(1.5f32,2.5f64);let copied=pair.clone();copied.0}}impl Value for S{}trait Unused:Sized{fn unused(self)->f64{let x=3.5f64;x.clone()}}fn main(){if (S{x:0}).value()==1.5f32{println!("defaults ok");}}"#,
    );
}
#[test]
fn declared_sized_self_aggregate_defaults() {
    valid(
        r#"struct S{x:i32}trait V:Sized{fn wrap(self)->(Self,){(self,)}fn empty(&self)->i32{let values:[Self;0]=[];7}}impl V for S{}fn main(){let s=(S{x:3}).wrap().0;if s.x==3&&s.empty()==7{println!("defaults ok");}}"#,
    );
    valid(
        r#"struct S{x:i32}trait Base:Sized{}trait V:Base{fn wrap(self)->(Self,){(self,)}}impl Base for S{}impl V for S{}fn main(){if (S{x:3}).wrap().0.x==3{println!("defaults ok");}}"#,
    );
}
#[test]
fn unsized_self_aggregate_default_is_invalid() {
    invalid("trait V{fn value(&self){let values:[Self;0]=[];}}fn main(){}");
}
#[test]
fn unused_wrong_result_is_invalid() {
    invalid("trait Value:Sized{fn value(self)->i32{true}}fn main(){}");
}
#[test]
fn unsized_self_static_value_positions_are_invalid() {
    for source in [
        "trait V{fn value(x:Self){}}fn main(){}",
        "trait V{fn value()->Self{loop{}}}fn main(){}",
    ] {
        invalid(source);
    }
}
#[test]
fn unsized_self_local_storage_is_invalid_even_when_diverging() {
    for source in [
        "trait V{fn value(&self){let x:Self=loop{};}}fn main(){}",
        "struct S{x:i32}trait V{fn value(&self){let x:Self=loop{};}}impl V for S{fn value(&self){}}fn main(){}",
        "trait V{fn value(&self){let x:Self;}}fn main(){}",
    ] {
        invalid(source);
    }
}
#[test]
fn unused_missing_name_is_invalid() {
    invalid("trait Value:Sized{fn value(self)->i32{missing()}}fn main(){}");
}
#[test]
fn unused_noncopy_duplication_is_invalid() {
    invalid("trait Duplicate:Sized{fn duplicate(self)->(Self,Self){(self,self)}}fn main(){}");
}
#[test]
fn overridden_invalid_default_stays_invalid() {
    invalid(
        "struct S{x:i32}trait Value:Sized{fn value(self)->i32{missing()}}impl Value for S{fn value(self)->i32{7}}fn main(){}",
    );
}
#[test]
fn overridden_invalid_default_move_stays_invalid() {
    invalid(
        "struct S{x:i32}trait Value:Sized{fn required(self)->i32;fn value(self)->i32{self.required()+self.required()}}impl Value for S{fn required(self)->i32{self.x}fn value(self)->i32{7}}fn main(){}",
    );
}
#[test]
fn overridden_invalid_default_loan_stays_invalid() {
    invalid(
        "struct S{x:i32}trait Value:Sized{fn required(&mut self)->i32;fn value(&mut self)->i32{let a=&mut *self;let b=&mut *self;a.required()+b.required()}}impl Value for S{fn required(&mut self)->i32{self.x}fn value(&mut self)->i32{7}}fn main(){}",
    );
}
#[test]
fn missing_required_item_is_invalid() {
    invalid(
        "struct S{x:i32}trait Value:Sized{fn required(self)->i32;fn value(self)->i32{7}}impl Value for S{}fn main(){}",
    );
}
#[test]
fn conflicting_default_items_are_invalid() {
    invalid("trait Value:Sized{fn value(self)->i32{7}fn value(self)->i32{8}}fn main(){}");
}
#[test]
fn ambiguous_default_dispatch_is_invalid() {
    invalid(
        "struct S{x:i32}trait A:Sized{fn value(self)->i32{7}}trait B:Sized{fn value(self)->i32{8}}impl A for S{}impl B for S{}fn main(){let x=S{x:1}.value();}",
    );
}
