//! Inherited associated members preserve their declaring supertrait identity.
use justrust_engine::compiler;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

#[test]
fn generic_supertrait_projection_runs() {
    let source = r#"
trait Ix<I> { type Output; fn index(&self, i: I) -> &Self::Output; }
trait Im<I>: Ix<I> { fn index_mut(&mut self, i: I) -> &mut Self::Output; }
struct P<T> { v: T }
impl<T> Ix<usize> for P<T> { type Output = T; fn index(&self, i: usize) -> &T { &self.v } }
impl<T> Im<usize> for P<T> { fn index_mut(&mut self, i: usize) -> &mut T { &mut self.v } }
fn main() { let mut p = P { v: 4 }; let r = p.index_mut(0); *r = 7; println!("{}", p.v); }
"#;
    for workers in [1, 8] {
        compiler::compile(source.into(), workers, true).unwrap();
        let compiled = compiler::compile(source.into(), workers, false).unwrap();
        let path =
            std::env::temp_dir().join(format!("jr-inherited-{}-{workers}", std::process::id()));
        fs::write(&path, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&path).output().unwrap();
        fs::remove_file(path).unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"7\n");
    }
}

#[test]
fn supertrait_diamond_preserves_one_owner() {
    let source = "trait A { type Item; } trait B: A {} trait C: A {} trait D: B + C { fn get(&self) -> &Self::Item; } struct S { v: i32 } impl A for S { type Item = i32; } impl B for S {} impl C for S {} impl D for S { fn get(&self) -> &i32 { &self.v } } fn main() {}";
    for workers in [1, 8] {
        compiler::compile(source.into(), workers, true).unwrap();
    }
    let ambiguous = "trait A { type Item; } trait C { type Item; } trait D: A + C { fn get(&self) -> &Self::Item; } fn main() {}";
    assert!(compiler::compile(ambiguous.into(), 1, true).is_err());
}

#[test]
fn inherited_projection_still_checks_return_type() {
    let source = "trait A { type Output; } trait B: A { fn get(&self) -> &Self::Output; } struct S { v: bool } impl A for S { type Output = i32; } impl B for S { fn get(&self) -> &bool { &self.v } } fn main() {}";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}
