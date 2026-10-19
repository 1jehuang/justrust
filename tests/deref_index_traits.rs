//! Overloaded `*x` (Deref) and `a[i]` (Index), and `&Self::Target`-style
//! projected reference returns, against stock rustc output.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn run(source: &str, expected: &str) {
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        let path = std::env::temp_dir().join(format!(
            "jr-deref-index-{}-{workers}-{}",
            std::process::id(),
            source.len()
        ));
        fs::write(&path, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&path).output().unwrap();
        let _ = fs::remove_file(&path);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "{source}"
        );
    }
}

#[test]
fn projected_reference_return() {
    run(
        "trait G { type T; fn get(&self) -> &Self::T; }
struct W { v: i32 }
impl G for W { type T = i32; fn get(&self) -> &i32 { &self.v } }
fn main() { let w = W { v: 5 }; let x = *w.get() + 1; println!(\"{}\", x); }",
        "6\n",
    );
}

#[test]
fn overloaded_deref_and_index() {
    run(
        "use std::ops::{Deref, Index};
struct W { v: i32 }
impl Deref for W { type Target = i32; fn deref(&self) -> &i32 { &self.v } }
struct A { a: [i32; 3] }
impl Index<usize> for A { type Output = i32; fn index(&self, i: usize) -> &i32 { if i == 1 { &self.a[1] } else { &self.a[2] } } }
fn main() { let w = W { v: 5 }; let x = *w + 1; let a = A { a: [1, 2, 3] }; let i: usize = 2; let s = a[1] + a[i]; let one = a[1]; println!(\"{} {} {}\", x, s, one); }",
        "6 5 2\n",
    );
}

#[test]
fn missing_deref_is_rejected() {
    let source =
        "struct W { v: i32 } fn main() { let w = W { v: 1 }; let x = *w; println!(\"{}\", x); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}
