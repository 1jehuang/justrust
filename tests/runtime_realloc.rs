//! Grow native collections past page boundaries. Compare observable values with
//! stock Rust, including owning elements whose representation must move whole.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::process::Command;

#[test]
fn raw_resize_failure_preserves_allocation() {
    let dir = std::env::temp_dir().join(format!("jr-raw-realloc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("main.rs");
    std::fs::write(
        &src,
        r#"
fn main() {
    let p = std::mem::raw_realloc(0usize, 0usize, 4096usize);
    assert!(p != 0usize);
    let ptr = p as *mut u64;
    unsafe { *ptr = 123456789u64; }
    let failed = std::mem::raw_realloc(p, 4096usize, 18446744073709551615usize);
    assert!(failed == 0usize);
    unsafe { assert!(*ptr == 123456789u64); }
    let grown = std::mem::raw_realloc(p, 4096usize, 1048576usize);
    assert!(grown != 0usize);
    let grown_ptr = grown as *const u64;
    unsafe { assert!(*grown_ptr == 123456789u64); }
    let shrunk = std::mem::raw_realloc(grown, 1048576usize, 4096usize);
    assert!(shrunk != 0usize);
    let shrunk_ptr = shrunk as *const u64;
    unsafe { assert!(*shrunk_ptr == 123456789u64); }
    let freed = std::mem::raw_realloc(shrunk, 4096usize, 0usize);
    assert!(freed == 0usize);
    println!("ok");
}
"#,
    )
    .unwrap();
    for workers in [1, 8] {
        let exe = dir.join(format!("native-{workers}"));
        let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("--workers")
            .arg(workers.to_string())
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        let actual = Command::new(&exe).output().unwrap();
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(actual.stdout, b"ok\n");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn collection_growth_preserves_values() {
    let dir = std::env::temp_dir().join(format!("jr-realloc-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("main.rs");
    std::fs::write(
        &src,
        r#"
fn main() {
    let mut values: Vec<i32> = Vec::<i32>::new();
    let mut i = 0i32;
    while i < 10000 {
        values.push(i);
        i += 1;
    }
    let mut total = 0i32;
    while let Some(n) = values.pop() { total += n; }
    println!("{} {}", total, values.len());
    let mut text = String::new();
    let mut j = 0usize;
    while j < 10000usize { text.push_str("abc"); j += 1usize; }
    println!("{}", text.len());
    let mut owned: Vec<String> = Vec::<String>::new();
    let mut k = 0usize;
    while k < 100usize { owned.push(String::from("hello")); k += 1usize; }
    let mut bytes = 0usize;
    while let Some(s) = owned.pop() { bytes += s.len(); }
    println!("{}", bytes);
}
"#,
    )
    .unwrap();
    let stock = dir.join("stock");
    let result = Command::new("rustc")
        .arg("--edition=2024")
        .arg(&src)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(result.status.success(), "{result:?}");
    let expected = Command::new(&stock).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        let exe = dir.join(format!("native-{workers}"));
        let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("--workers")
            .arg(workers.to_string())
            .arg("-o")
            .arg(&exe)
            .output()
            .unwrap();
        assert!(build.status.success(), "{build:?}");
        let actual = Command::new(&exe).output().unwrap();
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(actual.stdout, expected.stdout);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
