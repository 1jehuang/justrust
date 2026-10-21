//! Prelude slice methods (`swap`, `to_vec`, `contains`, `first`, `last`,
//! `is_empty`), `Vec::extend_from_slice` and Vec range views `&v[a..b]`.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use oracle::{Case, Expect, check};
use std::process::Command;

const POSITIVE: &str = r#"
fn f(v: &mut [i32]) { v.swap(0, 1); }
fn merge(v: &[i32]) -> Vec<i32> {
    if v.len() <= 1 { return v.to_vec(); }
    let mid = v.len() / 2;
    let mut out = Vec::new();
    out.extend_from_slice(&v[mid..]);
    out.extend_from_slice(&v[..mid]);
    out
}
fn main() {
    let mut a = vec![1, 2, 3, 4];
    f(a.as_mut_slice());
    let s: &[i32] = a.as_slice();
    let b = s.to_vec();
    let c = s.contains(&2);
    let d = s.contains(&9);
    let e = s.is_empty();
    let fi = s.first();
    let la = s.last();
    let x = b[0];
    println!("{} {} {} {} {:?} {:?}", x, c, d, e, fi, la);
    let t = &a[1..3];
    let n = t.len();
    let u = &a[..2];
    let m = u.to_vec();
    let k = m[1];
    let r = merge(&a[0..4]);
    let r0 = r[0];
    let empty: &[i32] = &a[4..];
    let z = empty.first();
    println!("{} {} {} {} {:?}", n, k, r0, empty.is_empty(), z);
}
"#;
const POSITIVE_OUT: &str = "2 true false false Some(2) Some(4)\n2 1 3 true None\n";

#[test]
fn slice_methods_match_stock() {
    check(
        "slice-methods",
        &[
            Case {
                name: "methods",
                source: POSITIVE,
                expect: Expect::Run(POSITIVE_OUT),
            },
            Case {
                name: "shared_swap_unused",
                source: "fn bad(s: &[i32]) { s.swap(0, 1); }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0596"),
            },
            Case {
                name: "shared_range_mut_unused",
                source: "fn bad(v: &Vec<i32>) { let s = &mut v[1..]; s[0] = 1; }\nfn main() { println!(\"x\"); }\n",
                expect: Expect::Invalid("E0596"),
            },
        ],
    );
}

fn scratch(tag: &str) -> std::path::PathBuf {
    let p = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-slice-methods-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn workers_one_and_eight_check_build_run() {
    for workers in ["1", "8"] {
        let dir = scratch(workers);
        let src = dir.join("main.rs");
        std::fs::write(&src, POSITIVE).unwrap();
        let check = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .args(["--check", "--workers", workers])
            .output()
            .unwrap();
        assert!(
            check.status.success(),
            "{}",
            String::from_utf8_lossy(&check.stderr)
        );
        let image = dir.join("native");
        let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
            .arg(&src)
            .arg("-o")
            .arg(&image)
            .args(["--workers", workers])
            .output()
            .unwrap();
        assert!(
            build.status.success(),
            "{}",
            String::from_utf8_lossy(&build.stderr)
        );
        let ran = Command::new(&image).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&ran.stdout), POSITIVE_OUT);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// Out-of-bounds `swap` panics with stock's index message under panic=abort.
#[test]
fn swap_out_of_bounds_panics_like_stock() {
    let source = "fn f(v: &mut [i32]) { v.swap(0, 5); }\nfn main() { let mut a = vec![1, 2, 3]; println!(\"before\"); f(a.as_mut_slice()); println!(\"after\"); }\n";
    let dir = scratch("panic");
    let src = dir.join("main.rs");
    std::fs::write(&src, source).unwrap();
    let image = dir.join("native");
    let build = Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg(&src)
        .arg("-o")
        .arg(&image)
        .output()
        .unwrap();
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let native = Command::new(&image).output().unwrap();
    let stock_image = dir.join("stock");
    let stock = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
        .arg(&src)
        .arg("-o")
        .arg(&stock_image)
        .output()
        .unwrap();
    assert!(stock.status.success());
    let stock = Command::new(&stock_image).output().unwrap();
    assert_eq!(native.stdout, stock.stdout);
    assert_eq!(native.status.code(), stock.status.code());
    let message = "index out of bounds: the len is 3 but the index is 5";
    assert!(String::from_utf8_lossy(&native.stderr).contains(message));
    assert!(String::from_utf8_lossy(&stock.stderr).contains(message));
    let _ = std::fs::remove_dir_all(&dir);
}
