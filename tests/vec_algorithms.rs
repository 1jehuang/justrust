//! Source-library Vec algorithms compared with stock Rust at workers 1 and 8.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn compare(source: &str, expected: &str, tag: &str) {
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let root = base.join(format!("jr-vec-algorithms-{}-{tag}", std::process::id()));
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
        let built = compiler::compile(source.into(), workers, false).unwrap();
        let binary = root.join(format!("native-{workers}"));
        fs::write(&binary, built.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let result = Command::new(&binary).output().unwrap();
        assert!(result.status.success(), "{result:?}");
        assert_eq!(result.stdout, stock.stdout);
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vec_comparison_algorithms() {
    compare(
        r#"
fn main() {
    let mut values: Vec<i32> = Vec::new();
    values.push(3); values.push(1); values.push(2); values.push(2); values.push(3);
    values.sort(); values.dedup();
    println!("{} {} {} {}", values.len(), values[0], values[1], values[2]);
    println!("{} {}", values.contains(&2), values.contains(&4));
    match values.binary_search(&2) { Ok(i) => println!("found {}", i), Err(i) => println!("missing {}", i) }
    match values.binary_search(&4) { Ok(i) => println!("found {}", i), Err(i) => println!("missing {}", i) }
    let mut empty: Vec<i32> = Vec::new();
    empty.sort(); empty.dedup();
    println!("{} {}", empty.len(), empty.contains(&0));
}
"#,
        "3 1 2 3\ntrue false\nfound 1\nmissing 3\n0 false\n",
        "integers",
    );
}

#[test]
fn vec_stable_noncopy_sort_and_dedup() {
    compare(
        r#"
struct Item { key: i32, order: i32 }
impl PartialEq for Item {
    fn eq(&self, other: &Item) -> bool { self.key == other.key }
}
impl Eq for Item {}
impl PartialOrd for Item {
    fn partial_cmp(&self, other: &Item) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Item {
    fn cmp(&self, other: &Item) -> std::cmp::Ordering { self.key.cmp(&other.key) }
}
fn main() {
    let mut values: Vec<Item> = Vec::new();
    values.push(Item { key: 2, order: 0 });
    values.push(Item { key: 1, order: 1 });
    values.push(Item { key: 2, order: 2 });
    values.push(Item { key: 1, order: 3 });
    values.sort();
    let a = &values[0]; let b = &values[1];
    let c = &values[2]; let d = &values[3];
    println!("{} {} {} {}", a.order, b.order, c.order, d.order);
    values.dedup();
    let first = &values[0]; let second = &values[1];
    println!("{} {} {}", values.len(), first.order, second.order);
    println!("{}", values.contains(&Item { key: 2, order: 99 }));
}
"#,
        "1 3 0 2\n2 1 0\ntrue\n",
        "noncopy",
    );
}
