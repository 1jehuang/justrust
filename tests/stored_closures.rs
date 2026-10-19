//! Stored closure CHECK, BUILD and runtime acceptance against stock Rust.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn stored_closures_check_build_and_run_at_both_worker_counts() {
    use std::os::unix::fs::PermissionsExt;
    let cases = [
        "fn main(){let f=|x:i32|x+1;println!(\"{}\",f(2));}",
        "fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)}fn main(){let f=|x:i32|x+1;println!(\"{}\",apply(f,2));}",
    ];
    for workers in [1, 8] {
        for (index, source) in cases.iter().enumerate() {
            for check in [true, false] {
                let compiled = justrust_engine::compiler::compile((*source).into(), workers, check)
                    .unwrap_or_else(|e| panic!("workers={workers} check={check}: {e}"));
                assert_eq!(compiled.executable.is_none(), check);
                let templates: std::collections::BTreeSet<_> = compiled
                    .report
                    .node_ids
                    .iter()
                    .zip(&compiled.report.node_evaluations)
                    .filter(|(_, (key, _))| key.kind == 110)
                    .map(|(id, (key, _))| {
                        assert!(
                            !key.params.is_empty(),
                            "closure template is not per function"
                        );
                        *id
                    })
                    .collect();
                assert!(
                    templates.len() >= 2,
                    "closure child has no independent body fact"
                );
                assert!(
                    compiled
                        .report
                        .dependency_edges
                        .iter()
                        .any(|(_, producer, slot)| { *slot == 1 && templates.contains(producer) }),
                    "closure child did not read its canonical parent capture/body output"
                );
                if check {
                    assert!(
                        compiled
                            .report
                            .node_evaluations
                            .iter()
                            .all(|(key, _)| !matches!(key.kind, 103 | 105)),
                        "CHECK demanded code generation"
                    );
                } else {
                    let path = std::env::temp_dir().join(format!(
                        "jr-stored-{}-{workers}-{index}",
                        std::process::id()
                    ));
                    std::fs::write(&path, compiled.executable.unwrap()).unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    let output = std::process::Command::new(&path).output().unwrap();
                    std::fs::remove_file(path).unwrap();
                    assert!(output.status.success(), "{output:?}");
                    assert_eq!(output.stdout, b"3\n");
                }
            }
        }
        for source in [
            "fn unused(){let f=|x:i32|->i32{true};}fn main(){}",
            "fn main(){let f=|x:i32|x+1;f(true);}",
            "fn main(){let f=||{let mut n=1i32;let r=&mut n;let q=r;*r};}",
            "fn main(){let f=|r:&mut i32|{let q=r;*r};}",
            "fn unused(r:&mut i32)->i32{let q=r;*r}fn main(){}",
        ] {
            assert!(justrust_engine::compiler::compile(source.into(), workers, true).is_err());
            assert!(justrust_engine::compiler::compile(source.into(), workers, false).is_err());
        }
    }
}

#[test]
fn noncapturing_values_match_stock() {
    oracle::check(
        "stored_closures",
        &[
            Case {
                name: "direct",
                source: "fn main(){let f=|x:i32|x+1; assert_eq!(f(2),3); assert_eq!(f(9),10); println!(\"stored\");}",
                expect: Expect::Run("stored\n"),
            },
            Case {
                name: "generic",
                source: "fn apply<F:Fn(i32)->i32>(f:F,x:i32)->i32{f(x)} fn main(){let f=|x:i32|x*2; assert_eq!(apply(f,21),42);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "borrowed_generic",
                source: "fn apply<F:Fn(i32)->i32>(f:&F,x:i32)->i32{f(x)} fn main(){let f=|x:i32|x*2; assert_eq!(apply(&f,21),42);assert_eq!(f(2),4);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "arity_zero_and_two",
                source: "fn main(){let z=||7;let add=|x:i32,y:i32|x+y;assert_eq!(z(),7);assert_eq!(add(2,3),5);}",
                expect: Expect::Run(""),
            },
            Case {
                name: "moved_unique_reference_in_stored_body",
                source: "fn main(){let f=||{let mut n=1i32;let r=&mut n;let q=r;*r};}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "moved_unique_reference_parameter_in_stored_body",
                source: "fn main(){let f=|r:&mut i32|{let q=r;*r};}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "moved_unique_reference_plain_body",
                source: "fn unused(r:&mut i32)->i32{let q=r;*r}fn main(){}",
                expect: Expect::Invalid("E0382"),
            },
            Case {
                name: "wrong_argument",
                source: "fn main(){let f=|x:i32|x+1; f(true);}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "bad_unused_body",
                source: "fn unused(){let f=|x:i32|->i32{true};} fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
