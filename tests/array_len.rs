//! Real compiler CLI acceptance for native array receiver adjustments.
use std::process::Command;

fn check_fixture(name: &str) {
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("array-len-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let input = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("tests/fixtures/array_len/{name}.rs"));
    let stock = root.join("stock");
    let built = Command::new("rustc")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(&input)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(built.status.success(), "{built:?}");
    let expected = Command::new(&stock).output().unwrap();
    assert!(expected.status.success(), "{expected:?}");
    for workers in ["1", "8"] {
        let native = root.join(format!("native-{workers}"));
        let check = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&input)
            .args(["--workers", workers, "--check"])
            .output()
            .unwrap();
        assert!(check.status.success(), "workers={workers}: {check:?}");
        assert!(!native.exists());
        let built = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&input)
            .args(["--workers", workers, "-o"])
            .arg(&native)
            .output()
            .unwrap();
        assert!(built.status.success(), "workers={workers}: {built:?}");
        let actual = Command::new(&native).output().unwrap();
        assert_eq!(actual.status.code(), expected.status.code());
        assert_eq!(actual.stdout, expected.stdout);
        assert_eq!(actual.stderr, expected.stderr);
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn array_receivers_match_stock_without_moves_or_lost_effects() {
    check_fixture("receivers");
}

#[test]
fn array_trait_priority_and_import_visibility_match_stock() {
    check_fixture("trait_priority");
}

#[test]
fn named_constant_and_empty_array_receivers_match_stock() {
    check_fixture("named_const");
    check_fixture("empty");
}
