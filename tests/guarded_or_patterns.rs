//! Guards on matches! or-patterns retry alternatives using ordinary match guards.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

/// Borrowed retry bindings use an all-path renewal proof without weakening B1.
#[test]
fn borrowed_guarded_or_workers1_8_check_build_run() {
    use std::{fs, process::Command};
    let directory = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("grouped-or-borrow-boundary-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.rs");
    fs::write(
        &source,
        include_str!("fixtures/patterns/guarded_borrowed_or.rs"),
    )
    .unwrap();
    let stock = directory.join("stock");
    let compiler = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    let built = Command::new(compiler)
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
        .arg(&source)
        .arg("-o")
        .arg(&stock)
        .output()
        .unwrap();
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let ran = Command::new(&stock).output().unwrap();
    assert!(ran.status.success());
    assert_eq!(ran.stdout, b"true 2\n");
    for workers in [1, 8] {
        for check in [true, false] {
            let image = directory.join(format!("native-{workers}-{check}"));
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(&source)
                .args(["--workers", &workers.to_string()]);
            if check {
                command.arg("--check");
            } else {
                command.arg("-o").arg(&image);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
            if check {
                assert!(!image.exists());
            } else {
                let ran = Command::new(&image).output().unwrap();
                assert!(ran.status.success());
                assert_eq!(ran.stdout, b"true 2\n");
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn guarded_or_workers1_8_check_build_run() {
    use std::{fs, process::Command};
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let directory = base.join(format!("grouped-or-workers-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.rs");
    fs::write(&source, r#"fn probe(c: &mut i32, value: bool) -> bool { *c += 1; value }
fn result(c: &mut i32) -> i32 { *c += 10; *c }
fn main() { let mut c = 0;
    let yes = matches!((true,false), (true,x)|(x,false) if probe(&mut c,x));
    let value = match (true,false) { (true,x)|(x,false) if probe(&mut c,x) => result(&mut c), _ => -1 };
    println!("{} {} {}", yes, value, c);
}"#).unwrap();
    for workers in [1, 8] {
        for check in [true, false] {
            let image = directory.join(format!("native-{workers}-{check}"));
            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
            command
                .arg(&source)
                .args(["--workers", &workers.to_string()]);
            if check {
                command.arg("--check");
            } else {
                command.arg("-o").arg(&image);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "workers={workers} check={check}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if check {
                assert!(!image.exists());
            } else {
                let ran = Command::new(&image).output().unwrap();
                assert!(ran.status.success());
                assert_eq!(ran.stdout, b"true 14 14\n");
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn guarded_or_patterns_match_stock() {
    oracle::check(
        "guarded-or-patterns",
        &[
            Case {
                name: "backtracking_bindings",
                source: r#"fn probe(c: &mut i32, value: bool) -> bool { *c += 1; value }
fn main() { let mut c = 0;
    let found = matches!((true, false), (true, x) | (x, false) if probe(&mut c, x));
    println!("{} {}", found, c);
}"#,
                expect: Expect::Run("true 2\n"),
            },
            Case {
                name: "guard_only_after_match",
                source: r#"fn main() { let mut c = 0;
    let no = matches!(3, 1 | 2 if { c += 1; true });
    let yes = matches!(2, 1 | 2 if { c += 1; true });
    println!("{} {} {}", no, yes, c);
}"#,
                expect: Expect::Run("false true 1\n"),
            },
            Case {
                name: "nested_guard_alternatives",
                source: r#"fn probe(c: &mut i32, value: bool) -> bool { *c += 1; value }
fn main() { let mut c = 0;
    let found = matches!(((true, false), 7), ((true, x) | (x, false), 7) if probe(&mut c, x));
    println!("{} {}", found, c);
}"#,
                expect: Expect::Run("true 2\n"),
            },
            Case {
                name: "ordinary_guard_and_body_calls",
                source: r#"fn probe(c: &mut i32, value: bool) -> bool { *c += 1; value }
fn output(c: &mut i32) -> i32 { *c += 10; *c }
fn main() { let mut c = 0;
    let value = match (true, false) {
        (true, x) | (x, false) if probe(&mut c, x) => output(&mut c),
        _ => -1,
    };
    println!("{} {}", value, c);
}"#,
                expect: Expect::Run("12 12\n"),
            },
            Case {
                name: "inconsistent_bindings",
                source: r#"fn main() { let _ = matches!(1, x | 2 if x > 0); }"#,
                expect: Expect::Invalid("E0408"),
            },
            Case {
                name: "shadowed_first_alternative_loop_exit",
                source: r#"fn main() { let mut calls = 0;
    let value = loop { match 2 {
        1 => {},
        1 | 2 if { calls += 1; true } => break 7,
        _ => continue,
    } };
    println!("{} {}", value, calls);
}"#,
                expect: Expect::Run("7 1\n"),
            },
        ],
    );
}
