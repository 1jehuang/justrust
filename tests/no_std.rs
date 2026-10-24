//! Crate-scoped no_std source policy, with a mixed std/no_std dependency graph.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::path::PathBuf;
use std::process::{Command, Output};

fn project(name: &str, library: &str) -> PathBuf {
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-no-std-{}-{name}", std::process::id()));
    for dir in ["src", "dep/src", "ordinary/src"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"client\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\ndep = { path = \"dep\" }\n").unwrap();
    std::fs::write(root.join("dep/Cargo.toml"), "[package]\nname = \"dep\"\nversion = \"0.1.0\"\nedition = \"2024\"\n[dependencies]\nordinary = { path = \"../ordinary\" }\n").unwrap();
    std::fs::write(
        root.join("ordinary/Cargo.toml"),
        "[package]\nname = \"ordinary\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("ordinary/src/lib.rs"),
        "pub fn value() -> i32 { let s = String::from(\"abc\"); s.len() as i32 }\n",
    )
    .unwrap();
    std::fs::write(root.join("dep/src/lib.rs"), library).unwrap();
    std::fs::write(
        root.join("src/main.rs"),
        "fn main() { println!(\"{}\", dep::value()); }\n",
    )
    .unwrap();
    root
}
fn native(root: &PathBuf, command: &str, workers: &str) -> Output {
    Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .args([command, "-j", workers])
        .current_dir(root)
        .env("CARGO_TARGET_DIR", root.join("native-target"))
        .output()
        .unwrap()
}
#[test]
fn mixed_crates_keep_defining_preludes_and_explicit_core_paths() {
    let root = project(
        "mixed",
        "#![no_std]\nmod external;\nmod nested { pub fn value() -> i32 { let x: core::option::Option<i32> = Some(super::external::value()); match x { Some(x) => x, None => 0i32 } } } pub fn value() -> i32 { nested::value() }\n",
    );
    std::fs::write(root.join("dep/src/external.rs"), "pub fn value() -> i32 { let x: core::option::Option<i32> = Some(ordinary::value()); match x { Some(x) => x, None => 0i32 } }\n").unwrap();
    let stock = Command::new("cargo")
        .args(["run", "--offline", "--quiet"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("stock-target"))
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    assert_eq!(stock.stdout, b"3\n");
    for workers in ["1", "8"] {
        for command in ["check", "build", "run"] {
            let out = native(&root, command, workers);
            assert!(
                out.status.success(),
                "{command}/{workers}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            if command == "run" {
                assert_eq!(out.stdout, stock.stdout);
            }
        }
    }
}
#[test]
fn no_std_rejects_unavailable_names_and_io_in_unused_items() {
    for (name, body) in [
        (
            "std",
            "pub fn unused() { let x = std::mem::size_of::<i32>(); }",
        ),
        ("vec", "pub fn unused() { let x = Vec::<i32>::new(); }"),
        ("string", "pub fn unused() { let x = String::from(\"x\"); }"),
        (
            "core-heap",
            "pub fn unused() { let x = core::vec::Vec::<i32>::new(); }",
        ),
        ("println", "pub fn unused() { println!(\"bad\"); }"),
        (
            "nested",
            "mod nested { pub fn unused() { let x = std::mem::size_of::<i32>(); } }",
        ),
        (
            "generated-io",
            "macro_rules! emit { () => { println!(\"bad\") } } pub fn unused() { emit!(); }",
        ),
    ] {
        let library = format!("#![no_std]\n{body}\npub fn value() -> i32 {{ 3i32 }}\n");
        let root = project(name, &library);
        for workers in ["1", "8"] {
            let out = native(&root, "check", workers);
            assert_eq!(
                out.status.code(),
                Some(101),
                "{name}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            assert!(!out.stderr.is_empty());
            assert!(!root.join("native-target/jr/debug/client").exists());
        }
    }
}
#[test]
fn no_std_root_executables_are_explicitly_unsupported() {
    let root = project("root-executable", "pub fn value() -> i32 { 3i32 }\n");
    std::fs::write(root.join("src/main.rs"), "#![no_std]\nfn main() {}\n").unwrap();
    for workers in ["1", "8"] {
        for command in ["check", "build"] {
            let out = native(&root, command, workers);
            assert_eq!(out.status.code(), Some(101));
            let diagnostic = String::from_utf8_lossy(&out.stderr);
            assert!(
                diagnostic.contains("unsupported")
                    && diagnostic.contains("no_std executable startup"),
                "{diagnostic}"
            );
            assert!(!root.join("native-target/jr/debug/client").exists());
        }
    }
}
#[test]
fn no_std_attribute_boundaries_are_explicit() {
    for (name, attribute) in [
        ("duplicate", "#![no_std]\n#![no_std]"),
        ("malformed", "#![no_std(x)]"),
        ("misplaced", "mod nested { #![no_std] }"),
        ("outer", "#[no_std] pub fn unused() {}"),
        ("opt-in", "#![no_std]\nextern crate std;"),
    ] {
        let root = project(
            name,
            &format!("{attribute}\npub fn value() -> i32 {{ 3i32 }}\n"),
        );
        let out = native(&root, "check", "1");
        assert_eq!(
            out.status.code(),
            Some(101),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(!out.stderr.is_empty());
    }
}
#[test]
fn extern_crate_core_alias_names_core_in_no_std_and_std_crates() {
    // Adler2's crate-root pattern: `std::` names core when std is disabled.
    for (name, attribute) in [("no-std", "#![no_std]\n"), ("std", "")] {
        let library = format!(
            "{attribute}extern crate core as std;\nmod inner {{ pub fn pick() -> u32 {{ std::cmp::max(3u32, 4u32) }} }}\nuse std::hash::Hasher;\npub struct H(u64);\nimpl Hasher for H {{ fn finish(&self) -> u64 {{ self.0 }} fn write(&mut self, b: &[u8]) {{ self.0 += b.len() as u64; }} }}\npub fn value() -> i32 {{ let mut h = H(u64::from(inner::pick())); h.write(&[1u8, 2u8]); h.finish() as i32 }}\n"
        );
        let root = project(&format!("alias-{name}"), &library);
        let stock = Command::new("cargo")
            .args(["run", "--offline", "--quiet"])
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", root.join("stock-target"))
            .output()
            .unwrap();
        assert!(
            stock.status.success(),
            "{name}: {}",
            String::from_utf8_lossy(&stock.stderr)
        );
        assert_eq!(stock.stdout, b"6\n");
        for workers in ["1", "8"] {
            let checked = native(&root, "check", workers);
            assert!(
                checked.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&checked.stderr)
            );
            let run = native(&root, "run", workers);
            assert!(
                run.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&run.stderr)
            );
            assert_eq!(run.stdout, b"6\n", "{name}");
        }
    }
}
#[test]
fn extern_crate_items_outside_the_native_libraries_are_explicit() {
    for (name, item, accepted_by_stock) in [
        ("alloc", "extern crate alloc;", true),
        ("missing", "extern crate not_a_crate;", false),
        (
            "alias-collision",
            "extern crate core as c;\npub struct c;",
            false,
        ),
    ] {
        let root = project(name, &format!("{item}\npub fn value() -> i32 {{ 3i32 }}\n"));
        let stock = Command::new("cargo")
            .args(["check", "--offline", "--quiet"])
            .current_dir(&root)
            .env("CARGO_TARGET_DIR", root.join("stock-target"))
            .output()
            .unwrap();
        assert_eq!(stock.status.success(), accepted_by_stock, "{name}");
        let out = native(&root, "check", "1");
        assert_eq!(
            out.status.code(),
            Some(101),
            "{name}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stderr = String::from_utf8_lossy(&out.stderr);
        if accepted_by_stock {
            assert!(stderr.contains("unsupported"), "{name}: {stderr}");
        }
    }
}
#[test]
fn builtin_derives_name_core_in_no_std_crates() {
    let root = project(
        "derives",
        "#![no_std]\nextern crate core as std;\n#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]\npub struct Pair { a: u16, b: u16 }\npub fn value() -> i32 { let p = Pair { a: 1, b: 2 }; let q = p; if p == q && Pair::default() < p { (p.a + q.b) as i32 } else { 0i32 } }\n",
    );
    let stock = Command::new("cargo")
        .args(["run", "--offline", "--quiet"])
        .current_dir(&root)
        .env("CARGO_TARGET_DIR", root.join("stock-target"))
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    assert_eq!(stock.stdout, b"3\n");
    for workers in ["1", "8"] {
        let run = native(&root, "run", workers);
        assert!(
            run.status.success(),
            "{}",
            String::from_utf8_lossy(&run.stderr)
        );
        assert_eq!(run.stdout, b"3\n");
    }
}
#[test]
fn std_io_is_unavailable_in_no_std_crates() {
    let root = project(
        "io",
        "#![no_std]\nuse std::io;\npub fn value() -> i32 { 3i32 }\n",
    );
    let out = native(&root, "check", "1");
    assert_eq!(
        out.status.code(),
        Some(101),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stderr).contains("no_std"));
}
