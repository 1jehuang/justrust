//! Stock differential coverage for ordinary Rust-source prelude methods.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn differential(source: &str) -> Output {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!(
        "jr-prelude-methods-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    let input = dir.join("input.rs");
    let stock = dir.join("stock");
    let native = dir.join("native");
    fs::write(&input, source).unwrap();
    let output = Command::new("rustc")
        .args([
            "--edition=2024",
            "-Cpanic=abort",
            "--crate-name=prelude_methods",
        ])
        .arg(&input)
        .arg("-o")
        .arg(&stock)
        .output()
        .expect("stock rustc oracle must be installed");
    assert!(
        output.status.success(),
        "stock rejected source: {}\n{source}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = Command::new(&stock).output().unwrap();
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("native rejected source: {error}\n{source}"));
        fs::write(&native, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&native, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(&native).output().unwrap();
        assert_eq!(actual.status.code(), expected.status.code(), "{source}");
        assert_eq!(actual.stdout, expected.stdout, "{source}");
        if expected.status.success() {
            assert_eq!(actual.stderr, expected.stderr, "{source}");
        }
    }
    fs::remove_dir_all(&dir).unwrap();
    expected
}

#[test]
fn panicking_unwrap_matches_stock_exit() {
    for source in [
        "fn main() { let value: Option<i32> = None; value.unwrap(); }",
        "fn main() { let value: Result<i32, bool> = Err(true); value.unwrap(); }",
    ] {
        let expected = differential(source);
        assert!(!expected.status.success());
    }
}

#[test]
fn option_and_result_methods_match_stock() {
    let output = differential(
        r#"
fn main() {
    let some: Option<i32> = Some(7);
    if some.is_some() && !some.is_none() { println!("some"); }
    if some.unwrap_or(0) == 7 { println!("value"); }
    let none: Option<i32> = None;
    if none.is_none() && !none.is_some() { println!("none"); }
    if none.unwrap_or(4) == 4 { println!("default"); }
    let ok: Result<i32, bool> = Ok(9);
    if ok.is_ok() && !ok.is_err() { println!("ok"); }
    if ok.unwrap_or(0) == 9 { println!("value"); }
    let err: Result<i32, bool> = Err(true);
    if err.is_err() && !err.is_ok() { println!("err"); }
    if err.unwrap_or(5) == 5 { println!("default"); }
}
"#,
    );
    assert!(output.status.success());
}

#[test]
fn result_constructor_alias_paths_and_user_shadowing_match_stock() {
    differential(
        r#"
fn main() {
    let ok: std::result::Result<i32, bool> = core::result::Result::Ok(3);
    match ok { Ok(value) => if value == 3 { println!("path"); }, Err(_) => {} }
}
"#,
    );
    differential(
        r#"
enum Result { Mine }
mod mine { pub fn Ok(x: i32) -> i32 { x + 1 } }
use mine::Ok;
fn main() {
    let x = Result::Mine;
    match x { Result::Mine => println!("shadow") }
    if Ok(2) == 3 { println!("import"); }
}
"#,
    );
}

#[test]
fn predicates_borrow_noncopy_values_and_defaults_are_eager() {
    differential(
        r#"
struct Value { n: i32 }
fn default() -> Value { println!("default evaluated"); Value { n: 5 } }
fn main() {
    let value: Option<Value> = Some(Value { n: 8 });
    if value.is_some() && !value.is_none() { println!("borrow"); }
    let taken = value.unwrap_or(default());
    if taken.n == 8 { println!("moved"); }
    let result: Result<Value, Value> = Err(Value { n: 4 });
    if result.is_err() && !result.is_ok() { println!("borrow err"); }
    let fallback = result.unwrap_or(default());
    if fallback.n == 5 { println!("fallback"); }
}
"#,
    );
}
