//! Postfix `?` on Option/Result, checked against stock rustc.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

#[test]
fn static_boxed_errors_match_stock() {
    oracle::check(
        "static-boxed-errors",
        &[
            Case {
                name: "concrete_generic_method_keeps_sized_payload",
                source: r#"#[derive(Debug)] struct E;
impl std::fmt::Display for E { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "sized payload") } }
impl std::error::Error for E {}
struct Maker; impl Maker { fn boxed<T>(&self, value: T) -> Box<T> { Box::new(value) } }
fn main() { let error: Box<dyn std::error::Error> = Maker.boxed(E); println!("{}", error); }"#,
                expect: Expect::Run("sized payload\n"),
            },
            Case {
                name: "generic_error_preserves_static_bound",
                source: r#"#[derive(Debug)] struct E;
impl std::fmt::Display for E { fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "generic error") } }
impl std::error::Error for E {}
fn erase<E: std::error::Error + 'static>(error: E) -> Box<dyn std::error::Error> { error.into() }
fn main() { println!("{}", erase(E)); }"#,
                expect: Expect::Run("generic error\n"),
            },
            Case {
                name: "unused_generic_error_requires_static",
                source: "fn unused<E: std::error::Error>(error: E) -> Box<dyn std::error::Error> { error.into() } fn main() {}",
                expect: Expect::Invalid("lifetime"),
            },
            Case {
                name: "owned_messages_and_residual_error",
                source: r#"#[derive(Debug)] struct E(i32);
impl std::fmt::Display for E {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "custom {}", self.0) }
}
impl std::error::Error for E {}
impl Drop for E { fn drop(&mut self) { println!("drop {}", self.0); } }
fn fail() -> Result<i32, E> { Err(E(7)) }
fn forwarded() -> Result<i32, Box<dyn std::error::Error>> { Ok(fail()?) }
fn main() {
    let message: Box<dyn std::error::Error>;
    { let local = String::from("borrowed message"); message = local.as_str().into(); }
    println!("{} {:?}", message, message);
    let owned: Box<dyn std::error::Error> = String::from("owned message").into();
    println!("{} {:?}", owned, owned);
    match forwarded() { Ok(v) => println!("ok {}", v), Err(e) => println!("{} {:?}", e, e) }
}"#,
                expect: Expect::Run(
                    "borrowed message \"borrowed message\"\nowned message \"owned message\"\ncustom 7 E(7)\ndrop 7\n",
                ),
            },
            Case {
                name: "unused_missing_error_implementation",
                source: "struct NotError; fn unused() -> Box<dyn std::error::Error> { NotError.into() } fn main() {}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_borrowed_error_cannot_escape",
                source: r#"#[derive(Debug)] struct Borrowed<'a>(&'a str);
impl std::fmt::Display for Borrowed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}
impl std::error::Error for Borrowed<'_> {}
fn unused<'a>(text: &'a str) -> Box<dyn std::error::Error> { Borrowed(text).into() }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
        ],
    );
}

#[test]
#[ignore = "non-static dyn object lifetime representation and payload source-carrier transport are not implemented"]
fn boxed_errors_match_stock() {
    oracle::check(
        "boxed-errors",
        &[
            Case {
                name: "owned_messages_and_residual_error",
                source: r#"#[derive(Debug)] struct E(i32);
impl std::fmt::Display for E {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "custom {}", self.0) }
}
impl std::error::Error for E {}
impl Drop for E { fn drop(&mut self) { println!("drop {}", self.0); } }
fn fail() -> Result<i32, E> { Err(E(7)) }
fn forwarded() -> Result<i32, Box<dyn std::error::Error>> { Ok(fail()?) }
fn main() {
    let message: Box<dyn std::error::Error>;
    { let local = String::from("borrowed message"); message = local.as_str().into(); }
    println!("{} {:?}", message, message);
    let owned: Box<dyn std::error::Error> = String::from("owned message").into();
    println!("{} {:?}", owned, owned);
    match forwarded() { Ok(v) => println!("ok {}", v), Err(e) => println!("{} {:?}", e, e) }
}"#,
                expect: Expect::Run(
                    "borrowed message \"borrowed message\"\nowned message \"owned message\"\ncustom 7 E(7)\ndrop 7\n",
                ),
            },
            Case {
                name: "borrowed_error_retains_object_lifetime",
                source: r#"#[derive(Debug)] struct Borrowed<'a>(&'a str);
impl std::fmt::Display for Borrowed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}
impl std::error::Error for Borrowed<'_> {}
fn borrowed<'a>(text: &'a str) -> Box<dyn std::error::Error + 'a> { Borrowed(text).into() }
fn main() {
    let text = String::from("borrowed error");
    let error = borrowed(text.as_str());
    println!("{} {:?}", error, error);
}"#,
                expect: Expect::Run("borrowed error Borrowed(\"borrowed error\")\n"),
            },
            Case {
                name: "unused_missing_error_implementation",
                source: "struct NotError; fn unused() -> Box<dyn std::error::Error> { NotError.into() } fn main() {}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_borrowed_error_cannot_escape",
                source: r#"#[derive(Debug)] struct Borrowed<'a>(&'a str);
impl std::fmt::Display for Borrowed<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "{}", self.0) }
}
impl std::error::Error for Borrowed<'_> {}
fn unused<'a>(text: &'a str) -> Box<dyn std::error::Error> { Borrowed(text).into() }
fn main() {}"#,
                expect: Expect::Invalid("lifetime"),
            },
        ],
    );
}

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
                name: "generic_operand_success_context_and_distinct_residual",
                source: r#"enum AppError { Parse(std::num::ParseIntError), Negative(i64), Empty }
impl From<std::num::ParseIntError> for AppError {
    fn from(e: std::num::ParseIntError) -> Self { AppError::Parse(e) }
}
fn parse_pos(s: &str) -> Result<i64, AppError> {
    let s = s.trim();
    if s.is_empty() { return Err(AppError::Empty); }
    let n: i64 = s.parse()?;
    if n < 0 { Err(AppError::Negative(n)) } else { Ok(n) }
}
struct Source;
impl Source { fn value<T: Default>(&self) -> Option<T> { Some(T::default()) } }
fn optional() -> Option<u16> { let n: u16 = Source.value()?; Some(n + 2) }
fn main() {
    for text in [" 12 ", "-3", "x", ""] {
        match parse_pos(text) {
            Ok(n) => println!("ok {}", n),
            Err(AppError::Parse(_)) => println!("parse"),
            Err(AppError::Negative(n)) => println!("negative {}", n),
            Err(AppError::Empty) => println!("empty"),
        }
    }
    println!("{:?}", optional());
}"#,
                expect: Expect::Run("ok 12\nnegative -3\nparse\nempty\nSome(2)\n"),
            },
            Case {
                name: "unused_try_missing_residual_conversion",
                source: r#"struct E;
fn unused(s: &str) -> Result<i64, E> { let n: i64 = s.parse()?; Ok(n) }
fn main() {}"#,
                expect: Expect::Invalid("E0271"),
            },
            Case {
                name: "unused_try_conflicting_success_type",
                source: r#"fn unused(s: &str) -> Result<i64, std::num::ParseIntError> {
    let n: i64 = s.parse::<u8>()?; Ok(n)
}
fn main() {}"#,
                expect: Expect::Invalid("E0308"),
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

#[test]
fn result_sum_matches_stock() {
    oracle::check(
        "result-sum",
        &[
            Case {
                name: "generic_success_and_first_error",
                source: r#"use std::iter::Sum;
struct Total(i32);
impl Sum<i32> for Total {
    fn sum<I: Iterator<Item = i32>>(mut iter: I) -> Total {
        let mut total = 10;
        while let Some(n) = iter.next() { total += n; }
        Total(total)
    }
}
struct Items { index: usize }
impl Iterator for Items {
    type Item = Result<i32, i32>;
    fn next(&mut self) -> Option<Self::Item> {
        println!("poll {}", self.index);
        self.index += 1;
        if self.index == 1 { Some(Ok(2)) }
        else if self.index == 2 { Some(Err(7)) }
        else { panic!("polled after error") }
    }
}
impl Drop for Items { fn drop(&mut self) { println!("drop input"); } }
fn parse(s: &str) -> Result<i64, std::num::ParseIntError> { s.parse() }
fn sum_line(s: &str) -> Result<i64, std::num::ParseIntError> { s.split(',').map(parse).sum() }
fn main() {
    let success: Result<Total, i32> = vec![Ok(2), Ok(3)].into_iter().sum();
    match success { Ok(Total(n)) => println!("total {}", n), Err(e) => println!("err {}", e) }
    let empty: Vec<Result<i32, i32>> = Vec::new();
    let empty_sum: Result<i32, i32> = empty.into_iter().sum();
    println!("{:?}", empty_sum);
    let failure: Result<Total, i32> = Items { index: 0 }.sum();
    match failure { Ok(Total(n)) => println!("total {}", n), Err(e) => println!("err {}", e) }
    println!("{} {}", sum_line("1,2,3").unwrap(), sum_line("1,x,3").is_err());
}"#,
                expect: Expect::Run("total 15\nOk(0)\npoll 0\npoll 1\ndrop input\nerr 7\n6 true\n"),
            },
            Case {
                name: "generic_sum_bound_survives_unrelated_candidates",
                source: r#"use std::iter::Sum;
fn forward<T: Sum<U>, U, I: Iterator<Item = U>>(iter: I) -> T { T::sum(iter) }
fn qualified<T: Sum<U>, U, I: Iterator<Item = U>>(iter: I) -> T { <T as Sum<U>>::sum(iter) }
fn main() {
    let left: Vec<i32> = vec![2, 3];
    let right: Vec<i32> = vec![4, 5];
    println!("{} {}", forward::<i32, i32, _>(left.into_iter()), qualified::<i32, i32, _>(right.into_iter()));
}"#,
                expect: Expect::Run("5 9\n"),
            },
            Case {
                name: "unused_result_sum_without_success_sum",
                source: r#"struct NoSum;
fn unused(v: Vec<Result<NoSum, i32>>) -> Result<NoSum, i32> { v.into_iter().sum() }
fn main() {}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_closed_item_wrong_payload",
                source: r#"struct Bad;
impl Iterator for Bad {
    type Item = i32;
    fn next(&mut self) -> Option<Self::Item> { Some(true) }
}
fn main() {}"#,
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_result_sum_changed_error_type",
                source: r#"fn unused(v: Vec<Result<i32, u8>>) -> Result<i32, u16> { v.into_iter().sum() }
fn main() {}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "open_error_payload_inferred_through_into_iter",
                source: r#"fn main() {
    let s: Result<i32, i32> = vec![Ok(2), Ok(3)].into_iter().sum();
    println!("{:?}", s);
    let c: Result<Vec<i64>, String> = vec![Ok(4), Ok(5)].into_iter().collect();
    println!("{:?}", c);
}"#,
                expect: Expect::Run("Ok(5)\nOk([4, 5])\n"),
            },
            Case {
                name: "blanket_head_with_bound_implemented_for_nominal",
                source: r#"trait Shape { fn area(&self) -> u32; }
trait Marker {}
struct W<T>(T);
impl Marker for W<u16> {}
impl<P: Marker> Shape for P { fn area(&self) -> u32 { 1 } }
impl Shape for W<u8> { fn area(&self) -> u32 { 2 } }
fn main() {
    let a = W(5u8);
    let b = W(5u16);
    println!("{} {}", a.area(), b.area());
    let c = vec![W(7u16)];
    println!("{}", c.into_iter().map(|w| w.area()).sum::<u32>());
}"#,
                expect: Expect::Run("2 1\n1\n"),
            },
            Case {
                name: "blanket_head_bound_covers_nominal_stays_candidate",
                source: r#"trait Shape { fn area(&self) -> u32; }
trait Marker {}
struct W<T>(T);
impl Marker for W<u16> {}
impl<P: Marker> Shape for P { fn area(&self) -> u32 { 1 } }
impl Shape for W<u8> { fn area(&self) -> u32 { 2 } }
fn main() {
    let w = W(3);
    println!("{}", w.area());
}"#,
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "open_error_payload_mismatched_annotation",
                source: r#"fn main() {
    let s: Result<i32, i32> = vec![Ok(2), Err("x")].into_iter().sum();
    println!("{:?}", s);
}"#,
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
                name: "try_payload_inference",
                source: "fn twice(a: Option<i32>) -> Option<i32> { let x = a?; Some(x + x) }\nfn digit(c: char) -> Result<u32, u8> { let d = c.to_digit(10).ok_or(1u8)?; Ok(d + 1) }\nfn main() { println!(\"{:?} {:?} {:?}\", twice(Some(4)), digit('7'), digit('x')); }",
                expect: Expect::Run("Some(8) Ok(8) Err(1)\n"),
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

/// Stock-compile `source` and run it; return (stdout, stderr, exit code).
fn stock_run(source: &str, tag: &str) -> Option<(Vec<u8>, Vec<u8>, Option<i32>)> {
    let dir = std::env::temp_dir().join(format!("jr-try-stock-{}-{tag}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("main.rs");
    let image = dir.join("stock");
    std::fs::write(&input, source).unwrap();
    let built = std::process::Command::new(oracle::pinned_stock::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort", "-Awarnings"])
        .arg(&input)
        .arg("-o")
        .arg(&image)
        .output()
        .unwrap();
    let result = built.status.success().then(|| {
        let ran = std::process::Command::new(&image).output().unwrap();
        (ran.stdout, ran.stderr, ran.status.code())
    });
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn native_run(source: &str, workers: usize, tag: &str) -> (Vec<u8>, Vec<u8>, Option<i32>) {
    let dir = std::env::temp_dir().join(format!(
        "jr-try-native-{}-{tag}-{workers}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("main.rs");
    let image = dir.join("out");
    std::fs::write(&input, source).unwrap();
    let checked = std::process::Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg(&input)
        .args(["--workers", &workers.to_string(), "--check"])
        .output()
        .unwrap();
    assert!(checked.status.success(), "CHECK {tag}: {checked:?}");
    let built = std::process::Command::new(env!("CARGO_BIN_EXE_jr-rust"))
        .arg(&input)
        .args(["--workers", &workers.to_string()])
        .arg("-o")
        .arg(&image)
        .output()
        .unwrap();
    assert!(built.status.success(), "BUILD {tag}: {built:?}");
    let ran = std::process::Command::new(&image).output().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (ran.stdout, ran.stderr, ran.status.code())
}

/// `fn main() -> T: Termination`: `Err(e)` prints `Error: {e:?}` to stderr and
/// exits 1; `ExitCode` sets the status; `Ok`/`()` exit 0.
#[test]
fn fallible_main_reports_through_termination() {
    let cases = [
        (
            "err_debug",
            "#[derive(Debug)] struct E { code: i32 }\nfn run(x: i32) -> Result<i32, E> { if x > 2 { Err(E { code: x }) } else { Ok(x) } }\nfn main() -> Result<(), E> { println!(\"{}\", run(1)?); run(5)?; println!(\"unreachable\"); Ok(()) }",
        ),
        (
            "ok_string",
            "fn main() -> Result<(), String> { println!(\"hi\"); Ok(()) }",
        ),
        (
            "err_string",
            "fn main() -> Result<(), String> { let v = vec![1]; let x = v.get(3).ok_or(\"missing\".to_string())?; println!(\"{}\", x); Ok(()) }",
        ),
        (
            "boxed_parse_error",
            "fn main() -> Result<(), Box<dyn std::error::Error>> { let a: i64 = \"12\".parse()?; println!(\"{}\", a); let b: i64 = \"zz\".parse()?; println!(\"{}\", b); Ok(()) }",
        ),
        (
            "exit_code",
            "use std::process::ExitCode;\nfn main() -> ExitCode { println!(\"x\"); ExitCode::from(3) }",
        ),
        (
            "drops_before_report",
            "struct D; impl Drop for D { fn drop(&mut self) { println!(\"drop\"); } }\nfn main() -> Result<(), &'static str> { let _d = D; Err(\"bad\") }",
        ),
    ];
    for (tag, source) in cases {
        let expected = stock_run(source, tag).unwrap_or_else(|| panic!("stock rejected {tag}"));
        for workers in [1, 8] {
            assert_eq!(
                native_run(source, workers, tag),
                expected,
                "{tag} workers={workers}"
            );
        }
    }
    oracle::check(
        "fallible-main-invalid",
        &[
            Case {
                name: "error_without_debug",
                source: "struct NoDebug;\nfn main() -> Result<(), NoDebug> { Ok(()) }",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "integer_main",
                source: "fn main() -> i32 { 0 }",
                expect: Expect::Invalid("E0277"),
            },
        ],
    );
}

/// `Ok`/`Err`/`Some(Box::new(v))` under an expected `Box<dyn Trait>` payload
/// is an unsizing coercion site, not an equation on the pointee.
#[test]
fn boxed_trait_object_constructor_payloads_match_stock() {
    oracle::check(
        "boxed-object-constructor-payloads",
        &[
            Case {
                name: "err_box_new_to_box_dyn_error",
                source: r#"#[derive(Debug)] struct E;
impl std::fmt::Display for E { fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result { write!(f, "e") } }
impl std::error::Error for E {}
fn a(fail: bool) -> Result<i32, Box<dyn std::error::Error>> { if fail { return Err(Box::new(E)); } let n: i32 = "4".parse()?; Ok(n) }
fn main() { println!("{} {}", a(true).unwrap_err(), a(false).unwrap()); }"#,
                expect: Expect::Run("e 4\n"),
            },
            Case {
                name: "some_and_ok_box_new_to_box_dyn",
                source: r#"trait T { fn n(&self) -> i32; } struct A; impl T for A { fn n(&self) -> i32 { 3 } }
fn b() -> Option<Box<dyn T>> { Some(Box::new(A)) }
fn main() { let x: Result<Box<dyn T>, ()> = Ok(Box::new(A)); let b2 = Box::new(A); let o: Option<Box<dyn T>> = Some(b2); println!("{} {} {}", b().unwrap().n(), x.unwrap().n(), o.unwrap().n()); }"#,
                expect: Expect::Run("3 3 3\n"),
            },
            Case {
                name: "payload_missing_trait",
                source: "trait T {} trait U {} struct A; impl U for A {}\nfn f() -> Option<Box<dyn T>> { Some(Box::new(A)) }\nfn main() { f(); }",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "payload_wrong_concrete_box",
                source: "struct A; struct B;\nfn f() -> Option<Box<A>> { Some(Box::new(B)) }\nfn main() { f(); }",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}
