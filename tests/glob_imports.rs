//! Glob imports (`use m::*`, `use Enum::*`, `pub use` re-export globs,
//! `use super::*` in test modules): stock-differential run plus rejections.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use justrust_engine::compiler;

fn run(source: &str) -> String {
    let dir = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-glob-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let exe = dir.join("basic");
    let _ = std::fs::remove_file(&exe);
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false).unwrap();
        std::fs::write(&exe, compiled.executable.unwrap()).unwrap();
    }
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
    String::from_utf8(std::process::Command::new(&exe).output().unwrap().stdout).unwrap()
}

#[test]
fn glob_imports_match_stock() {
    let source = include_str!("fixtures/glob_imports/basic.rs");
    assert!(compiler::compile(source.into(), 1, true).is_ok());
    assert_eq!(run(source), "8 1 9 4 6\n");
}

#[test]
fn glob_imports_reject_private_ambiguous_and_unresolved() {
    for (source, message) in [
        (
            "mod a { fn h() -> i32 { 1 } } use a::*; fn main(){ let _ = h(); }",
            "unresolved function h",
        ),
        (
            "mod a { pub fn h() -> i32 { 1 } } mod b { pub fn h() -> i32 { 2 } } use a::*; use b::*; fn main(){ let _ = h(); }",
            "ambiguous",
        ),
        ("use nothere::*; fn main(){}", "unresolved glob import"),
    ] {
        let error = compiler::compile(source.into(), 1, true)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains(message), "{source}: {error}");
    }
    // Explicit items shadow glob imports.
    assert!(
        compiler::compile(
            "mod a { pub fn h() -> i32 { 1 } } mod b { pub fn h() -> i32 { 2 } } use a::*; use b::*; fn h() -> i32 { 3 } fn main(){ let _ = h(); }".into(),
            1,
            true
        )
        .is_ok()
    );
}
