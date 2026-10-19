//! Bare scalar Display acceptance against stock rustc as an external oracle.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{fs, path::PathBuf, process::Command};

#[path = "support/oracle.rs"]
mod oracle;

#[test]
fn scalar_format_place_borrows_and_projection_values() {
    use oracle::{Case, Expect};
    oracle::check(
        "format-places",
        &[
            Case {
                name: "local_borrow_conflict",
                source: "fn main(){let mut n=1;println!(\"{} {}\",n,{n=2;n});}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "field_borrow_conflict",
                source: "struct S{n:i32}fn main(){let mut s=S{n:1};println!(\"{} {}\",s.n,{s.n=2;s.n});}",
                expect: Expect::Invalid("E0506"),
            },
            Case {
                name: "values",
                source: "struct S{n:i32}fn main(){let s=S{n:42};let a=[-7i32,8];let x=9;let r=&x;println!(\"{} {} {} {}\",s.n,a[0],*r,{12});}",
                expect: Expect::Run("42 -7 9 12\n"),
            },
            Case {
                name: "unused_non_display",
                source: "struct S;fn unused(){println!(\"{}\",S);}fn main(){}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

#[test]
fn scalar_format_frozen_sources_match_stock() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/format_scalars");
    let scratch = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-scalar-format-{}", std::process::id()));
    fs::create_dir_all(&scratch).unwrap();
    let mut failures = Vec::new();
    for category in ["positive", "negative", "boundary"] {
        for entry in fs::read_dir(root.join(category)).unwrap() {
            let source = entry.unwrap().path();
            let name = source.file_stem().unwrap().to_str().unwrap();
            let native = scratch.join("native");
            let stock = scratch.join("stock");
            let _ = fs::remove_file(&native);
            let _ = fs::remove_file(&stock);
            let expected_live = category == "positive" || matches!(name, "str_arg" | "debug");
            let stock_build = Command::new("rustc")
                .args([
                    "--edition=2024",
                    "-Cpanic=abort",
                    "-Awarnings",
                    "--crate-name=oracle",
                ])
                .arg(&source)
                .arg("-o")
                .arg(&stock)
                .output()
                .expect("stock oracle installed (run locally)");
            assert_eq!(
                stock_build.status.success(),
                category != "negative",
                "stock {name}: {}",
                String::from_utf8_lossy(&stock_build.stderr)
            );
            for mode in ["check", "build"] {
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.arg(&source);
                if mode == "check" {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&native);
                }
                let result = command.output().unwrap();
                if result.status.success() != expected_live {
                    failures.push(format!(
                        "{category}/{name} {mode}: {}",
                        String::from_utf8_lossy(&result.stderr)
                    ));
                    continue;
                }
                if !expected_live {
                    assert!(!result.stderr.is_empty(), "silent rejection {name}");
                    assert!(!native.exists(), "rejection published image {name}");
                } else if mode == "build" {
                    let actual = Command::new(&native).output().unwrap();
                    let expected = Command::new(&stock).output().unwrap();
                    assert!(expected.status.success());
                    if !actual.status.success()
                        || actual.stdout != expected.stdout
                        || actual.stderr != expected.stderr
                    {
                        failures.push(format!("{name}: native {actual:?}, stock {expected:?}"));
                    }
                }
            }
        }
    }
    fs::remove_dir_all(scratch).unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
