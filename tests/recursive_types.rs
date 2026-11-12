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

/// A type applied to a pointer wrapper of itself (`Link<Holder>` inside
/// `Holder`, `Weak<Watched>` inside `Watched`). Metadata validation used to
/// recurse without bound here and overflow the compiler stack.
#[test]
fn self_applied_pointer_wrappers_validate_and_run() {
    runs("self_pointer_holder");
}

/// A parent/child tree whose links sit behind `RefCell`, with scalar `Cell`
/// fields in the same recursive type.
#[test]
fn recursive_types_with_interior_mutability_run() {
    runs("refcell_tree");
}

/// Recursion through owned sequences and maps (`Vec<Self>`,
/// `BTreeMap<String, Self>`) is finite. Carrier inventory used to unfold it
/// until the planning depth limit.
#[test]
fn recursive_types_through_vec_and_btreemap_run() {
    runs("vec_btree_recursive");
}

#[test]
fn recursive_cell_methods_reject_invalid_unused_bodies() {
    for operation in ["first.marked.set(1)", "first.marked.set::<bool>(true)"] {
        let source = format!(
            "use std::cell::Cell; use std::rc::Rc; struct Node{{marked:Cell<bool>, next:Option<Rc<Node>>}} fn unused(nodes:Vec<Rc<Node>>){{let first=nodes[0].clone();{operation};}} fn main(){{}}"
        );
        for workers in [1, 8] {
            let dir = scratch();
            let input = dir.join("input.rs");
            let exe = dir.join("program");
            std::fs::write(&input, &source).unwrap();
            for build in [false, true] {
                let mut command = jr(&input, workers);
                if build {
                    command.arg("-o").arg(&exe);
                } else {
                    command.arg("--check");
                }
                let output = command.output().unwrap();
                assert!(!output.status.success(), "accepted {operation}: {output:?}");
                assert!(!exe.exists(), "published invalid executable");
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Only a primitive scalar `Cell` payload is supported. Inside a recursive
/// type any other payload is an explicit diagnostic, never an executable.
#[test]
fn recursive_cell_requires_scalar_payload() {
    let source = "use std::cell::Cell; use std::rc::Rc; struct S{next:Cell<Option<Rc<S>>>} fn main(){let s=S{next:Cell::new(None)};let _=s;}";
    for workers in [1, 8] {
        let dir = scratch();
        let input = dir.join("input.rs");
        std::fs::write(&input, source).unwrap();
        let exe = dir.join("program");
        for build in [false, true] {
            let mut command = jr(&input, workers);
            if build {
                command.arg("-o").arg(&exe);
            } else {
                command.arg("--check");
            }
            let out = command.output().unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "accepted: {stderr}");
            assert!(
                stderr.contains("unsupported: Cell payload requires a concrete primitive scalar"),
                "{stderr}"
            );
            assert!(!exe.exists(), "published executable");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn by_value_recursion_is_rejected() {
    for source in [
        "struct A{x:A} fn main(){}",
        "enum E{A(E),B} fn main(){}",
        "enum J{N,A(J)} fn main(){}",
        "struct A{x:(i32,[A;1])} fn main(){}",
        "struct A{x:[A;0]} fn main(){}",
        "struct A{x:Option<A>} fn main(){}",
        "struct A{b:B} struct B{a:A} fn main(){}",
        // Interior mutability is storage by value, not indirection.
        "use std::cell::RefCell; struct A{x:RefCell<A>} fn main(){}",
        "use std::cell::RefCell; struct A{x:RefCell<Option<A>>} fn main(){}",
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

/// References stored behind an owned `Box` have no tracked cell yet. Reading
/// them is an explicit unsupported diagnostic, never an invalid-program
/// verdict and never an executable. Lifetime errors on the same shapes are
/// still rejected.
#[test]
fn references_behind_box_are_explicitly_unsupported() {
    let unsupported = [
        "enum L<'a> { Nil, C(&'a str, Box<L<'a>>) } fn main() { let l = L::C(\"xy\", Box::new(L::C(\"abc\", Box::new(L::Nil)))); if let L::C(_, r) = &l { if let L::C(s, _) = &**r { println!(\"{s}\"); } } }",
        "enum L<'a> { Nil, C(&'a str, Box<L<'a>>) } fn len(l: &L) -> usize { match l { L::Nil => 0, L::C(_, r) => 1 + len(r) } } fn main() { let s = String::from(\"ab\"); let l = L::C(&s, Box::new(L::Nil)); println!(\"{}\", len(&l)); }",
    ];
    let invalid = [
        "enum L<'a> { Nil, C(&'a str, Box<L<'a>>) } fn main() { let s = String::from(\"ab\"); let l = L::C(\"x\", Box::new(L::C(&s, Box::new(L::Nil)))); drop(s); if let L::C(w, _) = &l { println!(\"{w}\"); } }",
        "fn main() { let s = String::from(\"ab\"); let b: Box<&str> = Box::new(&s); drop(s); let _keep = b; println!(\"done\"); }",
        "fn main() { let mut b: Box<&str> = Box::new(\"x\"); { let s = String::from(\"y\"); *b = &s; } println!(\"{}\", b); }",
        "enum L<'a> { Nil, C(&'a str, Box<L<'a>>) } fn main() { let mut l = L::C(\"x\", Box::new(L::Nil)); { let s = String::from(\"y\"); if let L::C(_, r) = &mut l { **r = L::C(&s, Box::new(L::Nil)); } } if let L::C(w, _) = &l { println!(\"{w}\"); } }",
        "fn mk<'a>() -> Box<&'a str> { let s = String::from(\"ab\"); Box::new(&s) } fn main() { let _ = mk(); }",
        "enum L<'a> { Nil, C(&'a str, Box<L<'a>>) } fn len(l: &L) -> usize { match l { L::Nil => 0, L::C(_, r) => 1 + len(r) } } fn unused() { let s = String::from(\"ab\"); let l = L::C(&s, Box::new(L::Nil)); let t = s; println!(\"{} {}\", len(&l), t); } fn main() {}",
        "enum L<'a> { Nil, C(&'a str, Box<L<'a>>) } fn unused() -> usize { let l; { let s = String::from(\"ab\"); l = L::C(&s, Box::new(L::Nil)); } match l { L::Nil => 0, L::C(w, _) => w.len() } } fn main() {}",
    ];
    for (source, must_be_unsupported) in unsupported
        .iter()
        .map(|s| (s, true))
        .chain(invalid.iter().map(|s| (s, false)))
    {
        for workers in [1, 8] {
            let dir = scratch();
            let input = dir.join("input.rs");
            std::fs::write(&input, source).unwrap();
            let exe = dir.join("program");
            for build in [false, true] {
                let mut command = jr(&input, workers);
                if build {
                    command.arg("-o").arg(&exe);
                } else {
                    command.arg("--check");
                }
                let out = command.output().unwrap();
                let stderr = String::from_utf8_lossy(&out.stderr);
                assert!(!out.status.success(), "accepted {source}: {stderr}");
                assert!(!exe.exists(), "published executable for {source}");
                if must_be_unsupported {
                    assert!(
                        stderr.to_ascii_lowercase().contains("unsupported"),
                        "{source}: {stderr}"
                    );
                    assert!(
                        !stderr.contains("no initialized issued provenance"),
                        "{source}: {stderr}"
                    );
                }
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// Static string data (no issued loan) may still be stored behind a Box.
#[test]
fn boxed_static_references_run() {
    runs("box_static_refs");
}

/// A trait bound on a recursive type's parameter is a well-formedness bound:
/// it never blocks layout, and it is still enforced where the type is used.
#[test]
fn bounded_recursive_type_runs() {
    runs("bounded_box_list");
}

#[test]
fn bounded_recursive_type_rejects_unsatisfied_bound() {
    for source in [
        "struct NoOrd; enum Tree<T: Ord> { Leaf, Node(Box<Tree<T>>, T, Box<Tree<T>>) } fn unused() { let _t: Tree<NoOrd> = Tree::Leaf; } fn main() {}",
        "struct W<T: Ord> { v: T, next: Option<Box<W<T>>> } fn unused(_w: W<f64>) {} fn main() {}",
        "struct W<T: Ord> { v: T, next: Option<Box<W<T>>> } struct Holder { w: Option<Box<W<f64>>> } fn unused(_h: Holder) {} fn main() {}",
    ] {
        for workers in [1, 8] {
            let dir = scratch();
            let input = dir.join("input.rs");
            std::fs::write(&input, source).unwrap();
            let out = jr(&input, workers).arg("--check").output().unwrap();
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(!out.status.success(), "accepted {source}: {stderr}");
            assert!(stderr.contains("E0277"), "{source}: {stderr}");
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}

/// A by-value `self` method on an owned Box receiver moves the pointee out
/// (`b.take()` is `(*b).take()`), consuming the Box.
#[test]
fn box_by_value_receiver_moves_out() {
    runs("box_by_value_receiver");
}

#[test]
fn box_by_value_receiver_rejects_reuse_and_shared_move() {
    for source in [
        "struct N { v: i32 } impl N { fn take(self) -> i32 { self.v } } fn unused() { let b = Box::new(N { v: 4 }); let n = b.take(); let m = b.take(); println!(\"{n} {m}\"); } fn main() {}",
        "struct N { v: i32 } impl N { fn take(self) -> i32 { self.v } } fn unused() { let b = Box::new(N { v: 4 }); let n = (*b).take(); let m = (*b).take(); println!(\"{n} {m}\"); } fn main() {}",
        "struct N { v: i32 } impl N { fn take(self) -> i32 { self.v } } fn unused(b: &Box<N>) -> i32 { b.take() } fn main() {}",
    ] {
        for workers in [1, 8] {
            let dir = scratch();
            let input = dir.join("input.rs");
            std::fs::write(&input, source).unwrap();
            let out = jr(&input, workers).arg("--check").output().unwrap();
            assert!(
                !out.status.success(),
                "accepted {source}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
