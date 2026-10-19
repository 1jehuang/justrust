//! Trait impls on `str` and reference Self types (`impl Tr for &T`), checked
//! against stock-matching output.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn run(source: &str, expected: &str) {
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("{error}\n{source}"));
        let dir = std::env::temp_dir().join(format!(
            "jr-trait-ref-self-{}-{workers}-{}",
            std::process::id(),
            source.len()
        ));
        fs::write(&dir, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).unwrap();
        let output = Command::new(&dir).output().unwrap();
        let _ = fs::remove_file(&dir);
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "{source}"
        );
    }
}

#[test]
fn str_self_impl() {
    run(
        "trait Shout { fn shout(&self) -> i32; }
         impl Shout for str { fn shout(&self) -> i32 { 7 } }
         fn main() { let s = \"hi\"; println!(\"{}\", s.shout()); println!(\"{}\", <str as Shout>::shout(s)); }",
        "7\n7\n",
    );
}

#[test]
fn reference_self_impls() {
    run(
        "trait Shout { fn shout(&self) -> i32; }
         impl Shout for &str { fn shout(&self) -> i32 { 8 } }
         fn main() { let s = \"a\"; println!(\"{}\", s.shout()); }",
        "8\n",
    );
    run(
        "trait Shout { fn shout(&self) -> i32; }
         impl Shout for i32 { fn shout(&self) -> i32 { *self + 1 } }
         impl<T: Shout> Shout for &T { fn shout(&self) -> i32 { (**self).shout() * 10 } }
         fn main() { let x = 5; let r = &x; println!(\"{}\", r.shout()); }",
        "6\n",
    );
}

#[test]
fn maybe_unsized_blanket_reference_impl() {
    run(
        "trait Shout { fn shout(&self) -> i32; }
         impl Shout for i32 { fn shout(&self) -> i32 { *self + 1 } }
         impl Shout for str { fn shout(&self) -> i32 { 7 } }
         impl<T: Shout + ?Sized> Shout for &T { fn shout(&self) -> i32 { (**self).shout() * 10 } }
         fn loud<T: ?Sized + Shout>(t: &T) -> i32 { t.shout() }
         fn main() { let x = 5; let r = &x; println!(\"{}\", r.shout()); println!(\"{}\", loud(&x)); }",
        "6\n6\n",
    );
}

#[test]
fn relaxing_other_bounds_is_rejected() {
    let source = "fn f<T: ?Copy>(t: T) {} fn main() {}";
    assert!(compiler::compile(source.into(), 1, true).is_err());
}

#[test]
fn dyn_dispatch_and_inferred_usize_index() {
    run(
        "trait Shape { fn area(&self) -> i32; fn name(&self) -> i32 { 1 } }
         struct Sq(i32);
         struct Rect { w: i32, h: i32 }
         impl Shape for Sq { fn area(&self) -> i32 { self.0 * self.0 } }
         impl Shape for Rect { fn area(&self) -> i32 { self.w * self.h } fn name(&self) -> i32 { 2 } }
         fn show(s: &dyn Shape) -> i32 { s.area() + s.name() }
         fn main() {
             let a = Sq(3);
             let b = Rect { w: 2, h: 5 };
             println!(\"{}\", show(&a));
             let list: [&dyn Shape; 2] = [&a, &b];
             let mut t = 0;
             let mut i = 0;
             while i < 2 { t += list[i].area(); i += 1; }
             println!(\"{}\", t);
         }",
        "10\n19\n",
    );
}
