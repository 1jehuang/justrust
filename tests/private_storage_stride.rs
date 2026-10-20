//! Compiler-private Box carrier extent is not a public Rust layout query.
use justrust_engine::compiler;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
#[path = "support/stock_rustc.rs"]
mod stock_rustc;

#[test]
fn private_stride_box_initialization_mutation_move_and_drop_match_stock() {
    let source = r#"
struct Pair { first: u64, second: u64 }
fn main() {
    {
        let mut value = Box::new(31u64);
        let before = *value.as_ref();
        *value.as_mut() = 47u64;
        let after = *value.as_ref();
        println!("{} {}", before, after);
    }
    let pair = Box::new(Pair { first: 11u64, second: 29u64 });
    let moved = pair;
    let record = moved.as_ref();
    let first = record.first;
    let second = record.second;
    println!("{} {}", first, second);
    {
        let narrow = Box::new(9u8);
        let wide = Box::new(123456789012345678901234567890u128);
        let narrow_value = *narrow.as_ref();
        let wide_value = *wide.as_ref();
        println!("{} {}", narrow_value, wide_value);
    }
    println!("dropped");
}
"#;
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-private-stride-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), source).unwrap();
    let compiled = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(root.join("main.rs"))
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    assert!(compiled.status.success(), "{compiled:?}");
    let expected = Command::new(root.join("stock")).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        let checked = compiler::compile(source.into(), workers, true)
            .unwrap_or_else(|error| panic!("CHECK {workers}: {error}"));
        assert!(checked.executable.is_none());
        assert!(!checked.report.published_kind(103));
        assert!(!checked.report.published_kind(137));
        let built = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|error| panic!("BUILD {workers}: {error}"));
        let path = root.join(format!("native-{workers}"));
        fs::write(&path, built.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(path).output().unwrap();
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(actual.stdout, expected.stdout, "workers={workers}");
        assert_eq!(actual.stderr, expected.stderr, "workers={workers}");
    }
    fs::remove_dir_all(root).unwrap();
}
