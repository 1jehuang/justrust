//! Postfix `?` on Option/Result, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn try_operator_matches_stock() {
    oracle::check(
        "try-operator",
        &[
            Case {
                name: "result_and_option",
                source: r#"enum E { Bad(i32) }
fn half(x: i32) -> Result<i32, E> { if x % 2 == 0 { Ok(x / 2) } else { Err(E::Bad(x)) } }
fn quarter(x: i32) -> Result<i32, E> { let h = half(x)?; Ok(half(h)? + 100) }
fn first(v: Option<i32>) -> Option<i32> { Some(v? + 1) }
fn chain(a: Option<i32>, b: Option<i32>) -> Option<i32> { let x = a?; let y = b?; Some(x * y) }
fn main() {
    match quarter(8) { Ok(v) => println!("{}", v), Err(E::Bad(b)) => println!("err {}", b) }
    match quarter(6) { Ok(v) => println!("{}", v), Err(E::Bad(b)) => println!("err {}", b) }
    match first(Some(3)) { Some(v) => println!("{}", v), None => println!("none") }
    match first(None) { Some(v) => println!("{}", v), None => println!("none") }
    match chain(Some(6), Some(7)) { Some(v) => println!("{}", v), None => println!("none") }
    match chain(Some(6), None) { Some(v) => println!("{}", v), None => println!("none") }
}"#,
                expect: Expect::Run("102\nerr 3\n4\nnone\n42\nnone\n"),
            },
            Case {
                name: "loop_early_exit",
                source: r#"fn sum(v: [i32; 4]) -> Result<i32, i32> {
    let mut total = 0;
    let mut i: usize = 0;
    while i < 4 { total += check(v[i])?; i += 1; }
    Ok(total)
}
fn check(x: i32) -> Result<i32, i32> { if x < 0 { Err(x) } else { Ok(x) } }
fn main() {
    match sum([1, 2, 3, 4]) { Ok(v) => println!("ok {}", v), Err(e) => println!("bad {}", e) }
    match sum([1, -2, 3, -4]) { Ok(v) => println!("ok {}", v), Err(e) => println!("bad {}", e) }
}"#,
                expect: Expect::Run("ok 10\nbad -2\n"),
            },
            Case {
                name: "try_in_unit_fn",
                source: r#"fn f(x: Option<i32>) -> i32 { x? }
fn main() { println!("{}", f(Some(1))); }"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "try_on_non_carrier",
                source: r#"fn f(x: i32) -> Option<i32> { Some(x?) }
fn main() { let _ = f(1); }"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "option_in_result_fn",
                source: r#"fn f(x: Option<i32>) -> Result<i32, i32> { Ok(x?) }
fn main() { let _ = f(Some(1)); }"#,
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

fn jr(source: &str, workers: usize, check: bool) -> (std::process::Output, Option<Vec<u8>>) {
    let dir = std::env::temp_dir().join(format!(
        "jr-try-{}-{workers}-{check}-{:x}",
        std::process::id(),
        source.len()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("main.rs");
    let image = dir.join("out");
    std::fs::write(&input, source).unwrap();
    let mut command = std::process::Command::new(env!("CARGO_BIN_EXE_jr-rust"));
    command
        .arg(&input)
        .args(["--workers", &workers.to_string()]);
    if check {
        command.arg("--check");
    } else {
        command.arg("-o").arg(&image);
    }
    let output = command.output().unwrap();
    let ran = (!check && output.status.success())
        .then(|| std::process::Command::new(&image).output().unwrap().stdout);
    assert!(!check || !image.exists(), "check published an executable");
    let _ = std::fs::remove_dir_all(&dir);
    (output, ran)
}

/// CHECK, BUILD and run at workers 1 and 8, including drops of live locals on
/// the `?` early-return path, plus rejection paths that must not publish.
#[test]
fn try_operator_workers_and_drops() {
    let drops = r#"struct D(i32);
impl Drop for D { fn drop(&mut self) { println!("drop {}", self.0); } }
fn step(x: i32) -> Result<i32, i32> { if x > 1 { Err(x) } else { Ok(x) } }
fn run(x: i32) -> Result<i32, i32> { let _a = D(10 + x); let v = step(x)?; let _b = D(20 + x); Ok(v) }
fn main() {
    match run(1) { Ok(v) => println!("ok {}", v), Err(e) => println!("err {}", e) }
    match run(5) { Ok(v) => println!("ok {}", v), Err(e) => println!("err {}", e) }
}"#;
    let expected = "drop 21\ndrop 11\nok 1\ndrop 15\nerr 5\n";
    oracle::check(
        "try-operator-drops",
        &[
            Case {
                name: "drops_on_early_return",
                source: drops,
                expect: Expect::Run(expected),
            },
            Case {
                name: "unused_bad_try",
                source: "fn unused(x: Option<i32>) -> i32 { x? }\nfn main() {}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
    let conversion = r#"#[derive(Debug)] struct A(i32);
#[derive(Debug)] enum B { Wrapped(i32), Other }
impl From<A> for B { fn from(a: A) -> B { println!("convert {}", a.0); B::Wrapped(a.0) } }
fn f(x: i32) -> Result<i32, A> { if x > 0 { Ok(x) } else { Err(A(x)) } }
fn g(x: i32) -> Result<i32, B> { if x == 7 { return Err(B::Other); } Ok(f(x)? + 1) }
fn main() {
    match g(3) { Ok(v) => println!("ok {}", v), Err(e) => println!("err {:?}", e) }
    match g(-4) { Ok(v) => println!("ok {}", v), Err(e) => println!("err {:?}", e) }
    match g(7) { Ok(v) => println!("ok {}", v), Err(e) => println!("err {:?}", e) }
}"#;
    let converted = "ok 4\nconvert -4\nerr Wrapped(-4)\nerr Other\n";
    oracle::check(
        "try-operator-from",
        &[
            Case {
                name: "from_conversion",
                source: conversion,
                expect: Expect::Run(converted),
            },
            Case {
                name: "unused_missing_from",
                source: "struct A; struct B;\nfn f() -> Result<i32, A> { Err(A) }\nfn unused() -> Result<i32, B> { Ok(f()?) }\nfn main() {}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_wrong_direction_from",
                source: "struct A; struct B;\nimpl From<B> for A { fn from(_: B) -> A { A } }\nfn f() -> Result<i32, A> { Err(A) }\nfn unused() -> Result<i32, B> { Ok(f()?) }\nfn main() {}",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
    for workers in [1, 8] {
        for check in [true, false] {
            for (source, expected) in [(drops, expected), (conversion, converted)] {
                let (output, ran) = jr(source, workers, check);
                assert!(
                    output.status.success(),
                    "workers={workers} check={check}: {output:?}"
                );
                if let Some(stdout) = ran {
                    assert_eq!(
                        String::from_utf8_lossy(&stdout),
                        expected,
                        "workers={workers}"
                    );
                }
            }
        }
    }
}
