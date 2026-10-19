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
         match y { Some(_) => println!(\"s\"), None => println!(\"n\") } }",
    );
    assert_eq!(out, "n\n");
}

#[test]
fn user_items_shadow_prelude_names() {
    let out = run("enum Option { A, B }\n\
         fn main() { let x = Option::A; match x { Option::A => println!(\"user\"), Option::B => println!(\"b\") } }");
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

#[test]
fn user_modules_and_imports_shadow_library_aliases() {
    assert_eq!(
        run(
            "mod std { pub mod convert { pub fn identity(x:i32)->i32{x+1} } } fn main(){ if std::convert::identity(1)==2 {println!(\"module\")} }"
        ),
        "module\n"
    );
    assert_eq!(
        run(
            "mod mine { pub fn identity(x:i32)->i32{x+2} } use mine as core; fn main(){if core::identity(1)==3 {println!(\"alias\")}}"
        ),
        "alias\n"
    );
}

#[test]
fn glob_imports_shadow_library_aliases_when_user_named() {
    assert_eq!(
        run(
            "mod mine { pub enum Option<T> { Some(T), None } } use mine::*; fn main(){ let x = Option::Some(1); match x { Option::Some(_) => println!(\"glob\"), Option::None => {} } }"
        ),
        "glob\n"
    );
}

#[test]
fn prelude_files_retain_original_source_provenance() {
    use compiler::source::{AssemblyStep, SourceAssembler, SourceOptions, prepare_text};
    use std::sync::Arc;
    let options = SourceOptions::default();
    let budget = compiler::token_primitives::ExpansionBudget::default();
    let mut assembler = SourceAssembler::new(&options);
    let mut step = assembler
        .provide(Arc::new(compiler::source::prepare_text_unit(
            "user.rs",
            "fn main() {}",
            &options,
        )))
        .unwrap();
    let prepared = loop {
        match step {
            AssemblyStep::Done(prepared) => break prepared,
            AssemblyStep::Need(need) => {
                let unit = match need.parent {
                    None => compiler::source::prepare_prelude_unit(&options, &budget),
                    Some(parent) => {
                        compiler::source::prepare_child_unit(&parent, need.index, &options, &budget)
                    }
                };
                step = assembler.provide(Arc::new(unit)).unwrap();
            }
        }
    };
    assert_eq!(prepared.files, 1);
    assert_eq!(prepared.source_bytes, "fn main() {}".len());
    let start = prepared.text.find("pub enum Option").unwrap();
    let span = prepared.map_range(start..start + 3).unwrap();
    let (path, line, _) = prepared.sources.location(span).unwrap();
    assert_eq!(path.to_str(), Some("<prelude>/__justrust_std/option.rs"));
    assert_eq!(line, 2);
    let disabled = SourceOptions {
        prelude: false,
        ..SourceOptions::default()
    };
    let user_only = prepare_text("user.rs", "fn main() {}", &disabled).unwrap();
    assert_eq!(user_only.sources.files.len(), 1);
}
