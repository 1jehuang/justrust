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
fn vec_index_format_borrows_noncopy_once() {
    compare(
        r#"
fn next(counter: &mut usize) -> usize {
    *counter += 1;
    0
}
fn main() {
    let mut words: Vec<String> = Vec::new();
    words.push(String::from("hello"));
    let mut count: usize = 0;
    println!("{}", words[next(&mut count)]);
    let text = format!("{}", words[next(&mut count)]);
    println!("{} {} {}", text, words[0], count);
}
"#,
        "hello\nhello hello 2\n",
        "noncopy",
    );
}

#[test]
fn vec_out_of_bounds_read_panics() {
    for source in [
        "fn main() { let mut v: Vec<i32> = Vec::new(); v.push(7); let i: usize = 1; let _x = v[i]; }",
        "fn main() { let mut v: Vec<i32> = Vec::new(); v.push(7); let i: usize = 1; v[i] = 9; }",
        "fn main() { let mut v: Vec<i32> = Vec::new(); v.push(7); let i: usize = 1; let _x = &mut v[i]; }",
    ] {
        for workers in [1, 8] {
            let compiled = compiler::compile(source.into(), workers, false).unwrap();
            let path = std::env::temp_dir()
                .join(format!("jr-vec-bounds-{}-{workers}", std::process::id()));
            fs::write(&path, compiled.executable.unwrap()).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            let result = Command::new(&path).output().unwrap();
            fs::remove_file(path).unwrap();
            assert_eq!(result.status.signal(), Some(6), "{result:?}");
        }
    }
}

#[test]
fn vec_mutable_index_places() {
    compare(
        r#"
fn main() {
    let mut values: Vec<i32> = Vec::new();
    values.push(11);
    values.push(22);
    let i: usize = 1;
    values[0] = 5;
    values[i] = values[0] + 3;
    values[0] += 7;
    values[i] *= 2;
    let element = &mut values[0];
    *element += 1;
    let other = &mut values[i];
    *other -= 2;
    println!("{} {}", values[0], values[i]);
}
"#,
        "13 14\n",
        "mutable",
    );
}

#[test]
fn vec_immutable_index_assignment_is_rejected() {
    for source in [
        "fn main() { let v: Vec<i32> = Vec::new(); v[0] = 1; }",
        "fn main() { let v: Vec<i32> = Vec::new(); v[0] += 1; }",
        "fn main() { let v: Vec<i32> = Vec::new(); let r = &mut v[0]; *r = 1; }",
    ] {
        for workers in [1, 8] {
            assert!(
                compiler::compile(source.into(), workers, true).is_err(),
                "{source}"
            );
            assert!(
                compiler::compile(source.into(), workers, false).is_err(),
                "{source}"
            );
        }
    }
}
