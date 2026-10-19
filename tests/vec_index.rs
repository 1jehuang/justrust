//! Ordinary source-library Vec indexing, compared with stock Rust.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::process::Command;

fn output(source: &str, workers: usize, tag: &str) -> Vec<u8> {
    let compiled = compiler::compile(source.into(), workers, false)
        .unwrap_or_else(|error| panic!("{error}\n{source}"));
    let path = std::env::temp_dir().join(format!(
        "jr-vec-index-{}-{workers}-{tag}",
        std::process::id()
    ));
    fs::write(&path, compiled.executable.unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    let result = Command::new(&path).output().unwrap();
    fs::remove_file(path).unwrap();
    assert!(result.status.success(), "{result:?}");
    result.stdout
}

fn compare(source: &str, expected: &str, tag: &str) {
    let root = std::env::temp_dir().join(format!("jr-vec-oracle-{}-{tag}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), source).unwrap();
    let stock = Command::new("rustc")
        .args(["--edition", "2024", "-C", "panic=abort"])
        .arg(root.join("main.rs"))
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    assert!(
        stock.status.success(),
        "{}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let stock = Command::new(root.join("stock")).output().unwrap();
    assert!(stock.status.success());
    assert_eq!(stock.stdout, expected.as_bytes());
    for workers in [1, 8] {
        compiler::compile(source.into(), workers, true).unwrap();
        assert_eq!(output(source, workers, tag), stock.stdout);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vec_shared_index_places() {
    compare(
        r#"
fn main() {
    let mut values: Vec<i32> = Vec::new();
    values.push(11);
    values.push(22);
    values.push(33);
    let i: usize = 2;
    let a = values[0];
    let b = values[i];
    let r = &values[1];
    let s = &values[i];
    println!("{} {} {} {} {}", a, b, r, s, values[1]);
    let text = format!("{}:{}", values[0], values[i]);
    println!("{}", text);
}
"#,
        "11 33 22 33 22\n11:33\n",
        "shared",
    );
}

#[test]
fn vec_out_of_bounds_read_panics() {
    let source = "fn main() { let mut v: Vec<i32> = Vec::new(); v.push(7); let i: usize = 1; let _x = v[i]; }";
    for workers in [1, 8] {
        let compiled = compiler::compile(source.into(), workers, false).unwrap();
        let path =
            std::env::temp_dir().join(format!("jr-vec-bounds-{}-{workers}", std::process::id()));
        fs::write(&path, compiled.executable.unwrap()).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::new(&path).output().unwrap();
        fs::remove_file(path).unwrap();
        assert_eq!(result.status.signal(), Some(6), "{result:?}");
    }
}
