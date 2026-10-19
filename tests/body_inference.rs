//! Body-local structural inference, compared with the stock compiler.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::process::Command;

fn compare(name: &str, source: &str) {
    let dir = std::env::temp_dir().join(format!("jr-body-inference-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("main.rs");
    std::fs::write(&src, source).unwrap();
    let stock = dir.join("stock");
    let out = Command::new("rustc")
        .args(["--edition=2024", "-Awarnings"])
        .arg(&src)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(out.status.success(), "stock {name}: {out:?}");
    let expected = Command::new(&stock).output().unwrap();
    for workers in [1, 8] {
        let exe = dir.join(format!("native-{workers}"));
        let _ = std::fs::remove_file(&exe);
        let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("--workers")
            .arg(workers.to_string())
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "native {name} workers {workers}: {out:?}"
        );
        let actual = Command::new(&exe).output().unwrap();
        assert_eq!(actual.status.code(), expected.status.code(), "{name}");
        assert_eq!(actual.stdout, expected.stdout, "{name}");
    }
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn later_receiver_uses_solve_generic_bindings() {
    compare(
        "push",
        "fn main() { let mut v = Vec::new(); v.push(1); v.push(2); println!(\"{}\", v.len()); }",
    );
    compare(
        "width",
        "fn consume(_: Vec<u8>) {} fn main() { let mut v = Vec::new(); v.push(255); consume(v); println!(\"ok\"); }",
    );
    compare(
        "nested",
        "fn main() { let mut outer = Vec::new(); let mut inner = Vec::new(); inner.push(7u8); outer.push(inner); println!(\"{}\", outer.len()); }",
    );
}

#[test]
fn expected_type_reaches_vec_macro_elements() {
    compare(
        "vec-macro",
        "fn consume(v: Vec<u8>) { println!(\"{}\", v.len()); } fn main() { consume(vec![1, 2]); }",
    );
}

#[test]
fn patterns_assignments_and_returns_solve_nominal_parameters() {
    compare(
        "option-assignment",
        "fn main() { let mut value = None; value = Some(7u8); match value { Some(n) => println!(\"{}\", n), None => {} } }",
    );
    compare(
        "pop-pattern",
        "fn main() { let mut values = Vec::new(); match values.pop() { Some(n) => { let typed: u8 = n; println!(\"{}\", typed); }, None => println!(\"empty\") } }",
    );
    compare(
        "trait-reference-pattern",
        "trait View { fn view(&self) -> &Option<u8>; } struct Holder { value: Option<u8> } impl View for Holder { fn view(&self) -> &Option<u8> { &self.value } } fn main() { let h = Holder { value: Some(7u8) }; match h.view() { Some(n) => { let typed: &u8 = n; println!(\"{}\", *typed); }, None => {} } }",
    );
    compare(
        "string-new",
        "fn main() { let mut s = String::new(); s.push('x'); println!(\"{}\", s.len()); }",
    );
    compare(
        "nested-return",
        "fn create() -> Vec<Vec<u16>> { let mut outer = Vec::new(); let mut inner = Vec::new(); inner.push(7); outer.push(inner); outer } fn main() { println!(\"{}\", create().len()); }",
    );
}

#[test]
fn contradictory_and_underconstrained_bindings_reject() {
    for (name, source) in [
        (
            "conflict",
            "fn main() { let mut v = Vec::new(); v.push(1u8); v.push(true); }",
        ),
        ("unknown", "fn main() { let v = Vec::new(); }"),
        (
            "unrelated-reference",
            "struct A { x: u8 } struct B { x: u8 } fn take(_: &B) {} fn main() { let a = A { x: 7 }; take(&a); }",
        ),
        (
            "wrong-deref-target",
            "struct A { x: u8 } struct B { x: u8 } fn take(_: &B) {} fn main() { let a = Box::new(A { x: 7 }); take(&a); }",
        ),
    ] {
        let dir =
            std::env::temp_dir().join(format!("jr-body-inference-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("main.rs");
        std::fs::write(&src, source).unwrap();
        let out = Command::new("rustc")
            .args(["--edition=2024", "--emit=metadata"])
            .arg(&src)
            .arg("-o")
            .arg(dir.join("stock.rmeta"))
            .output()
            .unwrap();
        assert!(!out.status.success(), "stock accepted {name}");
        for workers in [1, 8] {
            let exe = dir.join("native");
            let out = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
                .arg(&src)
                .arg("--workers")
                .arg(workers.to_string())
                .arg("-o")
                .arg(&exe)
                .output()
                .unwrap();
            assert!(!out.status.success(), "native accepted {name}");
            if name == "unknown" {
                assert!(
                    String::from_utf8_lossy(&out.stderr).contains("E0282: type annotations needed"),
                    "{out:?}"
                );
            }
            assert!(!exe.exists(), "failed inference published executable");
        }
        let _ = std::fs::remove_dir_all(dir);
    }
}
