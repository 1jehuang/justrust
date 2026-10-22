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

#[test]
fn box_deref() {
    run(
        "fn main() { let b = Box::new(5); let x = *b + 1; println!(\"{}\", x); }",
        "6\n",
    );
}

#[test]
fn deref_coercion_at_call_and_let_sites() {
    run(
        "struct N { v: i32 }
fn get(n: &N) -> i32 { n.v }
fn main() { let b = Box::new(N { v: 7 }); let bb = Box::new(Box::new(N { v: 2 })); let r: &N = &b; println!(\"{} {} {}\", get(&b), get(&bb), r.v); }",
        "7 2 7\n",
    );
}

#[test]
fn bounds_on_structs_and_enums() {
    run(
        "use std::fmt::Display;
struct Sorted<T: Ord> { v: T }
enum E<T> where T: Copy { A(T), B }
impl<T: Ord + Display> Sorted<T> { fn show(&self) -> &T { &self.v } }
fn main() { let s = Sorted { v: 3 }; let e: E<i32> = E::A(4); let n = match e { E::A(x) => x, E::B => 0 }; println!(\"{} {}\", s.show(), n); }",
        "3 4\n",
    );
    let bad = "struct S<T: Ord> { v: T }\nstruct F(f64);\nfn main() { let s = S { v: F(1.0) }; let _ = s; }";
    let error = compiler::compile(bad.into(), 1, true)
        .err()
        .expect("accepted");
    assert!(error.to_string().contains("E0277"), "{error}");
    let copy_bad = "struct S<T: Copy> { v: T } struct F { v: i32 } fn main() { let s = S { v: F { v: 1 } }; let _ = s; }";
    let error = compiler::compile(copy_bad.into(), 1, true)
        .err()
        .expect("accepted missing Copy bound");
    assert!(error.to_string().contains("E0277"), "{error}");
    let unused =
        "struct S<T: Copy> { v: T } struct F { v: i32 } fn unused(x: &S<F>) {} fn main() {}";
    let error = compiler::compile(unused.into(), 1, true)
        .err()
        .expect("accepted unused ill-formed signature");
    assert!(error.to_string().contains("E0277"), "{error}");
}

#[test]
fn explicit_deref_of_nominal_target() {
    // `*p`, `(*p).field`, `&*p` with a struct Target (stock: "4 4 4").
    run(
        "#[derive(Clone, Copy)]
struct W { v: i32 }
struct P { w: W }
impl std::ops::Deref for P { type Target = W; fn deref(&self) -> &W { &self.w } }
fn main() { let p = P { w: W { v: 4 } }; let c = *p; let f = (*p).v; let r = &*p; println!(\"{} {} {}\", c.v, f, r.v); }",
        "4 4 4\n",
    );
}

#[test]
fn deref_borrow_without_impl_is_rejected() {
    let source =
        "struct P { v: i32 } fn main() { let p = P { v: 1 }; let r = &*p; println!(\"{}\", r.v); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}

#[test]
fn method_and_field_autoderef_through_deref() {
    // Stock rustc 1.98.1: "4 4 9 9".
    run(
        "struct W { v: i32 }
impl W { fn get(&self) -> i32 { self.v } }
struct P { w: W }
impl std::ops::Deref for P { type Target = W; fn deref(&self) -> &W { &self.w } }
fn main() { let p = P { w: W { v: 4 } }; let a = p.get(); let b = p.v; let x = Box::new(W { v: 9 }); let c = x.get(); let d = x.v; println!(\"{} {} {} {}\", a, b, c, d); }",
        "4 4 9 9\n",
    );
}

#[test]
fn autoderef_without_member_is_rejected() {
    let source = "struct W { v: i32 }
struct P { w: W }
impl std::ops::Deref for P { type Target = W; fn deref(&self) -> &W { &self.w } }
fn main() { let p = P { w: W { v: 4 } }; let b = p.missing; println!(\"{}\", b); }";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}
