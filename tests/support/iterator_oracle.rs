use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
#[path = "stock_rustc.rs"]
mod stock_rustc;
static NEXT: AtomicU64 = AtomicU64::new(0);

fn root() -> std::path::PathBuf {
    std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!(
            "iterator-oracle-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ))
}

pub fn stock_native(source: &str) {
    stock_native_with_args(source, &[]);
}

pub fn stock_native_with_args(source: &str, args: &[&str]) {
    let root = root();
    fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    let stock = root.join("stock");
    fs::write(&input, source).unwrap();
    let built = Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "-Cpanic=abort"])
        .args(args)
        .arg(&input)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let expected = Command::new(&stock).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        let checked = compiler::compile(source.into(), workers, true)
            .unwrap_or_else(|e| panic!("CHECK workers={workers}: {e}\n{source}"));
        assert!(checked.executable.is_none());
        assert!(
            checked
                .report
                .node_evaluations
                .iter()
                .all(|(key, _)| key.kind != 103)
        );
        let built = compiler::compile(source.into(), workers, false)
            .unwrap_or_else(|e| panic!("BUILD workers={workers}: {e}\n{source}"));
        let path = root.join(format!("native-{workers}"));
        fs::write(&path, built.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(&path).output().unwrap();
        assert_eq!(actual.status.code(), expected.status.code());
        assert_eq!(actual.stdout, expected.stdout);
        assert_eq!(actual.stderr, expected.stderr);
    }
    fs::remove_dir_all(root).unwrap();
}

pub fn stock_and_native_reject(source: &str) {
    let root = root();
    fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    fs::write(&input, source).unwrap();
    let stock = Command::new(stock_rustc::stock_rustc())
        .args(["--edition=2024", "--emit=metadata"])
        .arg(&input)
        .arg("-o")
        .arg(root.join("out.rmeta"))
        .output()
        .unwrap();
    assert!(
        !stock.status.success(),
        "invalid control accepted by stock: {source}"
    );
    for workers in [1, 8] {
        for check in [true, false] {
            assert!(
                compiler::compile(source.into(), workers, check).is_err(),
                "invalid control accepted: {source}"
            );
        }
    }
    fs::remove_dir_all(root).unwrap();
}
