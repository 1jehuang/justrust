//! Native prelude library (PRELUDE.md): `libraries/prelude` loaded as per-file
//! SOURCE graph inputs behind `std::`/`core::` and the implicit prelude scope.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_ID: AtomicU64 = AtomicU64::new(0);

fn run(source: &str) -> String {
    let compiled = compiler::compile(source.into(), 1, false)
        .unwrap_or_else(|error| panic!("compile failed: {error}\n{source}"));
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = base.join(format!(
        "jr-prelude-{}-{epoch}-{}",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o700)
        .open(&path)
        .unwrap();
    file.write_all(&compiled.executable.unwrap()).unwrap();
    drop(file);
    let output = Command::new(&path).output().unwrap();
    let _ = fs::remove_file(&path);
    assert!(output.status.success(), "{source}");
    String::from_utf8(output.stdout).unwrap()
}

fn check_err(source: &str) -> String {
    match compiler::compile(source.into(), 1, true) {
        Ok(_) => panic!("expected rejection:\n{source}"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn prelude_option_names_resolve_without_imports() {
    let out = run(
        "fn get(x: i32) -> Option<i32> { if x > 0 { Some(x) } else { None } }\n\
         fn main() { match get(3) { Some(v) => if v == 3 { println!(\"some\") }, None => println!(\"none\") }\n\
         match get(0) { Some(_) => println!(\"bad\"), None => println!(\"none\") } }",
    );
    assert_eq!(out, "some\nnone\n");
}

#[test]
fn std_and_core_paths_reach_the_library() {
    let out = run(
        "fn main() { let y: std::option::Option<i32> = core::option::Option::None;\n\
         match y { Some(_) => println!(\"s\"), None => println!(\"n\") }\n\
         if core::convert::identity(5) == 5 { println!(\"id\") } }",
    );
    assert_eq!(out, "n\nid\n");
}

#[test]
fn user_items_shadow_prelude_names() {
    let out = run(
        "enum Option { A, B }\n\
         fn main() { let x = Option::A; match x { Option::A => println!(\"user\"), Option::B => println!(\"b\") } }",
    );
    assert_eq!(out, "user\n");
    let out = run(
        "mod m { pub fn Some(x: i32) -> i32 { x + 1 } }\nuse m::Some;\n\
         fn main() { if Some(1) == 2 { println!(\"import\") } }",
    );
    assert_eq!(out, "import\n");
}

#[test]
fn prelude_is_not_user_telemetry_or_user_names() {
    let compiled = compiler::compile("fn main() {}".into(), 1, true).unwrap();
    assert_eq!(compiled.functions, 1);
    assert_eq!(compiled.structs, 0);
    assert_eq!(compiled.enums, 0);
    // Library-internal modules are not in user scope.
    let error = check_err("fn main() { let x = option::Option::None::<i32>; }");
    assert!(!error.contains("unsupported"), "{error}");
}
