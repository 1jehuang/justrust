//! Recursive nominal types through indirection (`Box<Self>`, `Option<Box<Self>>`):
//! check, build and native run match stock rustc 1.98 (edition 2024,
//! -C panic=abort) stdout, including recursive drop order. By-value recursion
//! is rejected like stock E0072, even in unused declarations.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/recursive_types"
);

fn scratch() -> PathBuf {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let dir = base.join(format!(
        "jr-rec-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn jr(input: &Path, workers: usize) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    c.arg(input).args(["--workers", &workers.to_string()]);
    c
}

fn runs(name: &str) {
    let expected =
        std::fs::read_to_string(Path::new(FIXTURES).join(format!("{name}.stdout"))).unwrap();
    let input = Path::new(FIXTURES).join(format!("{name}.rs"));
    for workers in [1, 8] {
        let dir = scratch();
        let check = jr(&input, workers).arg("--check").output().unwrap();
        assert!(check.status.success(), "{name} check: {check:?}");
        let exe = dir.join("program");
        let build = jr(&input, workers).arg("-o").arg(&exe).output().unwrap();
        assert!(build.status.success(), "{name} build: {build:?}");
        let run = Command::new(&exe).output().unwrap();
        assert_eq!(run.status.code(), Some(0), "{name}: {run:?}");
        assert_eq!(String::from_utf8_lossy(&run.stdout), expected, "{name}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn boxed_recursive_enum_and_struct_run() {
    runs("box_list");
}

#[test]
fn boxed_recursive_tree_drops_in_stock_order() {
    runs("box_tree_drop");
}

#[test]
fn by_value_recursion_is_rejected() {
    for source in [
        "struct A{x:A} fn main(){}",
        "enum E{A(E),B} fn main(){}",
        "struct A{x:(i32,[A;1])} fn main(){}",
        "struct A{x:[A;0]} fn main(){}",
        "struct A{x:Option<A>} fn main(){}",
        "struct A{b:B} struct B{a:A} fn main(){}",
    ] {
        let dir = scratch();
        let input = dir.join("input.rs");
        std::fs::write(&input, source).unwrap();
        let out = jr(&input, 1).arg("--check").output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{source}: accepted");
        assert!(stderr.contains("infinite size"), "{source}: {stderr}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
