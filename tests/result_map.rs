//! Stock differential coverage for `Result::map` and `Result::map_err`.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

#[test]
fn result_map_and_map_err_match_stock() {
    use closure_oracle::{Case, Expect};
    closure_oracle::check(
        "result-map",
        &[
            Case {
                name: "map_transforms_ok_and_passes_err",
                source: r#"
fn main() {
    let a: Result<i32, bool> = Ok(4);
    let b: Result<i32, bool> = Err(true);
    let a2: Result<u64, bool> = a.map(|x| (x * 3) as u64);
    let b2: Result<u64, bool> = b.map(|x| (x * 3) as u64);
    println!("{} {}", a2.unwrap(), b2.unwrap_err());
}
"#,
                expect: Expect::Run("12 true\n"),
            },
            // A user struct produced by a generic closure result currently
            // gets two nominal identities in the existing checker (graph
            // realization vs source slot), the same failure 19's `fold`
            // hits. Scalar error types keep this case about map_err.
            Case {
                name: "map_err_changes_error_type_and_runs_only_on_err",
                source: r#"
fn main() {
    let a: Result<i32, u8> = Err(7);
    let b: Result<i32, u8> = Ok(5);
    let a2: Result<i32, i64> = a.map_err(|e| e as i64 * 100);
    let b2: Result<i32, i64> = b.map_err(|e| { println!("not called"); e as i64 });
    println!("{}", a2.unwrap_err());
    println!("{}", b2.unwrap());
}
"#,
                expect: Expect::Run("700\n5\n"),
            },
            Case {
                name: "map_err_composes_with_try",
                source: r#"
fn parse(ok: bool) -> Result<u32, u8> { if ok { Ok(9) } else { Err(2) } }
fn run(ok: bool) -> Result<u32, i64> {
    // `parse(ok).map_err(..)?` without this annotation is rejected by the
    // current checker ("unresolved method generic arguments"): it cannot
    // infer the closure output from the enclosing `?` conversion. The
    // semantic core resolves it (SEMANTIC-CORE.md); program 15 needs it.
    let mapped: Result<u32, i64> = parse(ok).map_err(|e| e as i64 - 10);
    let v = mapped?;
    Ok(v + 1)
}
fn main() {
    println!("{}", run(true).unwrap());
    println!("{}", run(false).unwrap_err());
}
"#,
                expect: Expect::Run("10\n-8\n"),
            },
            Case {
                name: "map_err_closure_cannot_take_ok_type",
                source: "fn main(){let r:Result<i32,u8>=Ok(1);let _=r.map_err(|e:i32| e);}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}
