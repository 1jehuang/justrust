//! Loan identities may renew only after the previous dynamic loan is dead.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn conditional_replacement_workers1_8_check_build_run() {
    use std::{fs, process::Command};
    let directory = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("loan-correlated-workers-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.rs");
    fs::write(
        &source,
        r#"fn main(){let mut value=0;let mut old=&mut value;let mut i=0;
while i<2{if i==0{old=&mut value;}*old+=1;i+=1;}println!("{}",*old);}"#,
    )
    .unwrap();
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
                assert_eq!(ran.stdout, b"2\n");
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

#[test]
fn loan_renewal_matches_stock() {
    let cases = [
        Case {
            name: "guard_failure_rebinds_payload",
            source: include_str!("fixtures/patterns/guarded_borrowed_or.rs"),
            expect: Expect::Run("true 2\n"),
        },
        Case {
            name: "guard_failure_inside_continue_break",
            source: r#"
enum E { Pair(bool, bool), Empty }
fn probe(c: &mut i32, value: bool) -> bool { *c += 1; value }
fn main() { let mut calls = 0; let mut count = 0; let mut i = 0;
    while i < 4 { i += 1;
        let yes = match E::Pair(true, false) {
            E::Pair(true, ref x) | E::Pair(ref x, false) if probe(&mut calls, *x) => true,
            _ => false,
        };
        if i == 2 { continue; }
        if i == 4 { break; }
        if yes { count += 1; }
    }
    println!("{} {}", calls, count);
}"#,
            expect: Expect::Run("8 2\n"),
        },
        Case {
            name: "nested_loop_reborrows",
            source: r#"
fn main() { let mut value = 0; let mut i = 0;
    'outer: while i < 3 { i += 1; let mut j = 0;
        while j < 4 { j += 1; let r = &mut value; *r += 1;
            if j == 1 { continue; }
            if i == 2 { continue 'outer; }
            if j == 3 { break; }
        }
    }
    println!("{}", value);
}"#,
            expect: Expect::Run("8\n"),
        },
        Case {
            name: "escaped_shared_loan_conflicts",
            source: r#"
fn main() { let mut value = 0; let mut old = &value; let mut i = 0;
    while i < 2 { value += 1; println!("{}", *old); old = &value; i += 1; }
    println!("{}", *old);
}"#,
            expect: Expect::Invalid("E0506"),
        },
        Case {
            name: "escaped_unique_loan_conflicts",
            source: r#"
fn main() { let mut value = 0; let mut old = &mut value; let mut i = 0;
    while i < 2 { let next = &mut value; *old += 1; old = next; i += 1; }
    println!("{}", *old);
}"#,
            expect: Expect::Invalid("E0499"),
        },
        Case {
            name: "branch_local_loan_is_killed_before_retry",
            source: r#"
fn main() { let mut value = 0; let mut i = 0;
    while i < 2 { if i == 0 { let local = &mut value; *local += 1; }
        else { let local = &mut value; *local += 1; } i += 1; }
    println!("{}", value);
}"#,
            expect: Expect::Run("2\n"),
        },
        Case {
            name: "old_loan_used_on_one_branch_after_reborrow",
            source: r#"
fn main() { let mut value = 0; let mut old = &mut value; let mut i = 0;
    while i < 2 { let next = &mut value; if i == 0 { *old += 1; }
        *next += 1; old = next; i += 1; }
    println!("{}", *old);
}"#,
            expect: Expect::Invalid("E0499"),
        },
        Case {
            name: "conditional_reference_replacement_preserves_validity",
            source: r#"
fn main() { let mut value = 0; let mut old = &mut value; let mut i = 0;
    while i < 2 { if i == 0 { old = &mut value; } *old += 1; i += 1; }
    println!("{}", *old);
}"#,
            expect: Expect::Run("2\n"),
        },
        Case {
            name: "transitive_reborrow_with_early_exit",
            source: r#"
fn main() { let mut value = 0; let parent = &mut value; let mut i = 0;
    loop { let child = &mut *parent; *child += 1; i += 1;
        if i == 3 { break; }
        if i == 1 { continue; }
    }
    println!("{}", *parent);
}"#,
            expect: Expect::Run("3\n"),
        },
        Case {
            name: "old_transitive_reborrow_survives_rebirth",
            source: r#"
fn main() { let mut value = 0; let parent = &mut value;
    let mut old = &mut *parent; let mut i = 0;
    while i < 2 { let next = &mut *parent; *old += 1; old = next; i += 1; }
    println!("{}", *old);
}"#,
            expect: Expect::Invalid("E0499"),
        },
        Case {
            name: "cross_branch_aliases_are_not_exclusive",
            source: r#"
fn main() { let mut value = 0; let first = &mut value;
    let flag = true; let second = if flag { &mut value } else { &mut value };
    *first += 1; *second += 1; println!("{}", value);
}"#,
            expect: Expect::Invalid("E0499"),
        },
        Case {
            name: "dangling_branch_reference_is_invalid",
            source: r#"
fn main() { let value = 0; let mut reference = &value;
    if true { let temporary = 1; reference = &temporary; }
    println!("{}", *reference);
}"#,
            expect: Expect::Invalid("E0597"),
        },
    ];
    let (positive, negative): (Vec<_>, Vec<_>) = cases
        .into_iter()
        .partition(|case| matches!(case.expect, Expect::Run(_)));
    oracle::check("loan-renewal", &positive);
    positive_loans(&positive);
    invalid_loans(&negative);
}

fn positive_loans(cases: &[Case]) {
    use std::{fs, process::Command};
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let directory = base.join(format!("loan-positive-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    for case in cases {
        let Expect::Run(expected) = case.expect else {
            panic!("positive fixture required")
        };
        let source = directory.join(format!("{}.rs", case.name));
        fs::write(&source, case.source).unwrap();
        for workers in [1, 8] {
            for check in [true, false] {
                let image = directory.join(format!("{}-{workers}-{check}", case.name));
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
                    "{} workers={workers} check={check}: {}",
                    case.name,
                    String::from_utf8_lossy(&output.stderr)
                );
                if check {
                    assert!(!image.exists(), "{}: CHECK published an image", case.name);
                } else {
                    let ran = Command::new(&image).output().unwrap();
                    assert!(ran.status.success(), "{}: native run failed", case.name);
                    assert_eq!(
                        ran.stdout,
                        expected.as_bytes(),
                        "{} workers={workers}",
                        case.name
                    );
                }
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}

// The public root preserves the entire mixed Analysis in Error::new, rather
// than assigning an Unsupported-only root verdict. An explicit complete
// BorrowConflict or InvalidReference is required here. Unsupported alone never counts as invalid.
fn definite_invalid(code: Option<i32>, stderr: &str, image_exists: bool) -> bool {
    code == Some(1)
        && !image_exists
        && stderr.contains("analysis (complete=true)")
        && (stderr.contains("kind: BorrowConflict(") || stderr.contains("kind: InvalidReference("))
}

#[test]
fn unsupported_only_is_not_an_invalid_loan_verdict() {
    assert!(!definite_invalid(
        Some(1),
        "analysis (complete=true): [Diagnostic { kind: UnsupportedBorrowing(\"SCC\") }]",
        false
    ));
    assert!(!definite_invalid(
        Some(1),
        "analysis (complete=false): [Diagnostic { kind: BorrowConflict(\"alias\") }]",
        false
    ));
    assert!(!definite_invalid(
        Some(1),
        "analysis (complete=true): [Diagnostic { kind: BorrowConflict(\"alias\") }]",
        true
    ));
}

fn invalid_loans(cases: &[Case]) {
    use std::{fs, process::Command};
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let directory = base.join(format!("loan-invalid-{}", std::process::id()));
    fs::create_dir_all(&directory).unwrap();
    let rustc = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    for case in cases {
        let source = directory.join(format!("{}.rs", case.name));
        fs::write(&source, case.source).unwrap();
        let stock_image = directory.join(format!("{}-stock", case.name));
        let stock = Command::new(&rustc)
            .env_remove("RUSTC_BOOTSTRAP")
            .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
            .arg(&source)
            .arg("-o")
            .arg(&stock_image)
            .output()
            .unwrap();
        let Expect::Invalid(code) = case.expect else {
            panic!("invalid fixture required")
        };
        assert!(
            !stock.status.success() && String::from_utf8_lossy(&stock.stderr).contains(code),
            "{}: {:?}",
            case.name,
            stock
        );
        for workers in [1, 8] {
            for check in [true, false] {
                let image = directory.join(format!("{}-{workers}-{check}", case.name));
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command
                    .arg(&source)
                    .args(["--workers", &workers.to_string()]);
                if check {
                    command.arg("--check");
                } else {
                    command.arg("-o").arg(&image);
                }
                let native = command.output().unwrap();
                let stderr = String::from_utf8_lossy(&native.stderr);
                eprintln!("{} workers={workers} check={check}: {stderr}", case.name);
                assert!(
                    definite_invalid(native.status.code(), &stderr, image.exists()),
                    "{}: expected complete canonical invalid diagnostic, not Unsupported alone: {stderr}",
                    case.name
                );
            }
        }
    }
    fs::remove_dir_all(directory).unwrap();
}
