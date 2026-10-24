//! Stock rustc is an external execution and rejection oracle only.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;
use oracle::{Case, Expect};

#[path = "support/closure_oracle.rs"]
mod closure_oracle;

fn parse_oracle(tag: &str, cases: &[closure_oracle::Case]) {
    closure_oracle::check_with_compiler(
        tag,
        cases,
        std::path::Path::new(&stock_rustc::stock_rustc()),
    );
}

#[test]
fn integer_radix_parsing_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "parse-radix",
        &[
            Case {
                name: "all_widths_and_radices",
                source: r#"fn main() {
                assert_eq!(i8::from_str_radix("-80", 16).unwrap(), i8::MIN);
                assert_eq!(i16::from_str_radix("-8000", 16).unwrap(), i16::MIN);
                assert_eq!(i32::from_str_radix("-80000000", 16).unwrap(), i32::MIN);
                assert_eq!(i64::from_str_radix("-8000000000000000", 16).unwrap(), i64::MIN);
                assert_eq!(i128::from_str_radix("-80000000000000000000000000000000", 16).unwrap(), i128::MIN);
                assert_eq!(isize::from_str_radix("-8000000000000000", 16).unwrap(), isize::MIN);
                assert_eq!(u8::from_str_radix("ff", 16).unwrap(), u8::MAX);
                assert_eq!(u16::from_str_radix("fFff", 16).unwrap(), u16::MAX);
                assert_eq!(u32::from_str_radix("FFFFFFFF", 16).unwrap(), u32::MAX);
                assert_eq!(u64::from_str_radix("ffffffffffffffff", 16).unwrap(), u64::MAX);
                assert_eq!(u128::from_str_radix("ffffffffffffffffffffffffffffffff", 16).unwrap(), u128::MAX);
                assert_eq!(usize::from_str_radix("ffffffffffffffff", 16).unwrap(), usize::MAX);
                for radix in 2u32..37u32 {
                    assert_eq!(u32::from_str_radix("10", radix).unwrap(), radix);
                    assert_eq!(i32::from_str_radix("-10", radix).unwrap(), -(radix as i32));
                    assert_eq!(u128::from_str_radix("+00010", radix).unwrap(), radix as u128);
                }
                assert_eq!(u32::from_str_radix("Zz", 36).unwrap(), 1295);
                assert_eq!(i32::from_str_radix("-0", 2).unwrap(), 0);
            }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "digits_signs_and_overflow",
                source: r#"use std::num::IntErrorKind;
            fn main() {
                for text in ["", "+", "-", "0x10", "1_0", " 10", "10 ", "é", "Ｆ", "2"] {
                    println!("{:?}", u8::from_str_radix(text, 2).unwrap_err().kind());
                }
                assert_eq!(i8::from_str_radix("80",16).unwrap_err().kind(), &IntErrorKind::PosOverflow);
                assert_eq!(i8::from_str_radix("-81",16).unwrap_err().kind(), &IntErrorKind::NegOverflow);
                assert_eq!(u8::from_str_radix("100",16).unwrap_err().kind(), &IntErrorKind::PosOverflow);
                assert_eq!(u128::from_str_radix("100000000000000000000000000000000",16).unwrap_err().kind(), &IntErrorKind::PosOverflow);
                assert_eq!(i128::from_str_radix("-80000000000000000000000000000001",16).unwrap_err().kind(), &IntErrorKind::NegOverflow);
                assert_eq!(u128::from_str_radix("-0",16).unwrap_err().kind(), &IntErrorKind::InvalidDigit);
                assert_eq!(u8::from_str_radix("z",35).unwrap_err().kind(), &IntErrorKind::InvalidDigit);
            }"#,
                expect: Expect::Run(
                    "Empty\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\nInvalidDigit\n",
                ),
            },
            Case {
                name: "unused_wrong_radix_type",
                source: "fn bad(){let _=u32::from_str_radix(\"1\", 16u64);}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn integer_radix_out_of_range_panics_like_stock() {
    use std::io::Read;
    use std::os::unix::process::{CommandExt, ExitStatusExt};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    unsafe extern "C" {
        fn prctl(option: std::ffi::c_int, ...) -> std::ffi::c_int;
    }
    let root = std::env::temp_dir().join(format!("jr-radix-panic-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    for radix in [0u32, 1, 37] {
        let source = root.join(format!("radix-{radix}.rs"));
        std::fs::write(
            &source,
            format!("fn main() {{ let _ = u32::from_str_radix(\"\", {radix}); }}"),
        )
        .unwrap();
        let message =
            format!("from_ascii_radix: radix must lie in the range `[2, 36]` - found {radix}");
        for workers in [0, 1, 8] {
            let image = root.join(format!("radix-{radix}-{workers}"));
            let mut command = if workers == 0 {
                let mut command = Command::new(stock_rustc::stock_rustc());
                command.args(["--edition=2024", "-Cpanic=abort", "--crate-name=radix"]);
                command
            } else {
                let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                command.args(["--workers", &workers.to_string()]);
                command
            };
            let built = command.arg(&source).arg("-o").arg(&image).output().unwrap();
            assert!(
                built.status.success(),
                "{}",
                String::from_utf8_lossy(&built.stderr)
            );
            let mut run = Command::new(&image);
            run.stdout(Stdio::null())
                .stderr(Stdio::piped())
                .env("RUST_BACKTRACE", "0");
            // SAFETY: only a libc syscall runs between fork and exec. Disabling
            // dumpability prevents a piped core service from delaying SIGABRT.
            unsafe {
                run.pre_exec(|| {
                    if prctl(4, 0usize, 0usize, 0usize, 0usize) == 0 {
                        Ok(())
                    } else {
                        Err(std::io::Error::last_os_error())
                    }
                });
            }
            let mut child = run.spawn().unwrap();
            let start = Instant::now();
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                if start.elapsed() > Duration::from_secs(10) {
                    let _ = child.kill();
                    let _ = child.wait();
                    panic!("radix panic timed out: {radix}, workers={workers}");
                }
                std::thread::sleep(Duration::from_millis(5));
            };
            let mut stderr = String::new();
            child
                .stderr
                .take()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            assert_eq!(status.signal(), Some(6), "{stderr}");
            assert!(stderr.contains(&message), "{stderr}");
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn bool_and_char_from_str_match_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "parse-bool-char",
        &[
            Case {
                name: "boolean_exact_spelling_and_errors",
                source: r#"use std::str::{FromStr, ParseBoolError};
fn parse<T: FromStr>(s: &str) -> Result<T, T::Err> { s.parse() }
fn main() {
    assert_eq!(bool::from_str("true").unwrap(), true);
    assert_eq!("false".parse::<bool>().unwrap(), false);
    let value: bool = parse::<bool>("true").unwrap();
    assert!(value);
    assert_eq!(String::from("false").parse::<bool>().unwrap(), false);
    for text in ["", "True", "FALSE", " true", "false ", "1", "0", "truefalse"] {
        let error: ParseBoolError = text.parse::<bool>().unwrap_err();
        assert_eq!(error, error.clone());
        println!("{} {:?}", error, error);
    }
}"#,
                expect: Expect::Run(concat!(
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                    "provided string was not `true` or `false` ParseBoolError\n",
                )),
            },
            Case {
                name: "unicode_scalar_not_byte_or_grapheme",
                source: r#"use core::str::FromStr;
use core::char::ParseCharError;
fn parse<T: FromStr>(s: &str) -> Result<T, T::Err> { s.parse() }
fn main() {
    assert_eq!(char::from_str("a").unwrap(), 'a');
    assert_eq!("é".parse::<char>().unwrap(), 'é');
    assert_eq!("🙂".parse::<char>().unwrap(), '🙂');
    let scalar: char = parse::<char>("中").unwrap();
    assert_eq!(scalar, '中');
    assert_eq!(" ".parse::<char>().unwrap(), ' ');
    assert_eq!("\0".parse::<char>().unwrap(), '\0');
    assert_eq!(String::from("é").parse::<char>().unwrap(), 'é');
    let empty: ParseCharError = "".parse::<char>().unwrap_err();
    assert_eq!(empty, empty.clone());
    println!("{} {:?}", empty, empty);
    for text in ["ab", "éa", "🙂🙂", "é"] {
        let error = text.parse::<char>().unwrap_err();
        println!("{} {:?}", error, error);
    }
}"#,
                expect: Expect::Run(concat!(
                    "cannot parse char from empty string ParseCharError { kind: EmptyString }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                    "too many characters in string ParseCharError { kind: TooManyChars }\n",
                )),
            },
            Case {
                name: "unused_incompatible_boolean_result",
                source: "fn bad()->Result<bool,core::char::ParseCharError>{\"true\".parse::<bool>()}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn integer_from_str_explicit_dispatch() {
    parse_oracle(
        "parse-explicit",
        &[
            closure_oracle::Case {
                name: "explicit_dispatch",
                source: "fn main() { let r: Result<i32, std::num::ParseIntError> = \"42\".parse::<i32>(); match r { Ok(n) => assert_eq!(n, 42), Err(_) => panic!(\"parse failed\") } }",
                expect: closure_oracle::Expect::Run(""),
            },
            closure_oracle::Case {
                name: "contextual_result",
                source: "fn main() { let r: Result<i32, std::num::ParseIntError> = \"42\".parse(); assert_eq!(r.unwrap(), 42); }",
                expect: closure_oracle::Expect::Run(""),
            },
            closure_oracle::Case {
                name: "explicit_chain",
                source: "fn main() { let n: i32 = \"42\".parse::<i32>().unwrap(); assert_eq!(n, 42); }",
                expect: closure_oracle::Expect::Run(""),
            },
            closure_oracle::Case {
                name: "contextual_chain",
                source: "fn main() { let n: i32 = \"42\".parse().unwrap(); assert_eq!(n, 42); }",
                expect: closure_oracle::Expect::Run(""),
            },
        ],
    );
}

#[test]
fn integer_from_str_receiver_forms() {
    for (name, source) in [
        (
            "narrow_context",
            "fn main(){let n:i8=\"-128\".parse().unwrap();assert_eq!(n,i8::MIN);}",
        ),
        (
            "wide_context",
            "use std::str::FromStr;fn main(){assert_eq!(i128::from_str(\"-170141183460469231731687303715884105728\").unwrap(),i128::MIN);}",
        ),
        (
            "owned_receiver",
            "fn main(){assert_eq!(String::from(\"+00042\").parse::<i32>().unwrap(),42);}",
        ),
        (
            "borrowed_owned_receiver",
            "fn main(){let s=String::from(\"42\");assert_eq!((&s).parse::<i32>().unwrap(),42);}",
        ),
    ] {
        parse_oracle(
            name,
            &[closure_oracle::Case {
                name,
                source,
                expect: closure_oracle::Expect::Run(""),
            }],
        );
    }
    parse_oracle(
        "parse-owned-drop",
        &[
            closure_oracle::Case {
                name: "owned_deref_temporary_drop_order",
                source: r#"
use std::ops::Deref;
struct Input { text: String }
impl Deref for Input {
    type Target = str;
    fn deref(&self) -> &str { self.text.as_str() }
}
impl Drop for Input { fn drop(&mut self) { println!("drop"); } }
fn make() -> Input { println!("make"); Input { text: String::from("42") } }
fn main() {
    let value = make().parse::<i32>().unwrap();
    println!("value {}", value);
}
"#,
                expect: closure_oracle::Expect::Run("make\ndrop\nvalue 42\n"),
            },
            closure_oracle::Case {
                name: "borrowed_result_cannot_escape_owned_deref_temporary",
                source: "fn make() -> String { String::from(\"one two\") } fn main() { let mut words = make().split_whitespace(); println!(\"{}\", words.next().unwrap()); }",
                expect: closure_oracle::Expect::Invalid("E0716"),
            },
        ],
    );
}

#[test]
fn integer_from_str_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "integer-from-str",
        &[
            Case {
                name: "all_integer_widths_and_context",
                source: r#"
use std::str::FromStr;
fn main() {
    let a: i8 = "-128".parse().unwrap(); assert_eq!(a, i8::MIN);
    let b: i16 = "32767".parse().unwrap(); assert_eq!(b, i16::MAX);
    let c: i32 = "-2147483648".parse().unwrap(); assert_eq!(c, i32::MIN);
    let d: i64 = "9223372036854775807".parse().unwrap(); assert_eq!(d, i64::MAX);
    let e: isize = "-9223372036854775808".parse().unwrap(); assert_eq!(e, isize::MIN);
    assert_eq!("255".parse::<u8>().unwrap(), u8::MAX);
    assert_eq!("65535".parse::<u16>().unwrap(), u16::MAX);
    assert_eq!("4294967295".parse::<u32>().unwrap(), u32::MAX);
    assert_eq!("18446744073709551615".parse::<u64>().unwrap(), u64::MAX);
    assert_eq!("18446744073709551615".parse::<usize>().unwrap(), usize::MAX);
    assert_eq!(i128::from_str("-170141183460469231731687303715884105728").unwrap(), i128::MIN);
    assert_eq!("170141183460469231731687303715884105727".parse::<i128>().unwrap(), i128::MAX);
    assert_eq!("340282366920938463463374607431768211455".parse::<u128>().unwrap(), u128::MAX);
    assert_eq!(String::from("+00042").parse::<i32>().unwrap(), 42);
    assert_eq!("-0".parse::<i8>().unwrap(), 0);
    assert_eq!("+0".parse::<u128>().unwrap(), 0);
}
"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "error_kinds_and_formatting",
                source: r#"
use std::num::{IntErrorKind, ParseIntError};
fn main() {
    let empty: ParseIntError = "".parse::<i32>().unwrap_err();
    assert_eq!(empty.kind(), &IntErrorKind::Empty);
    println!("{} {:?}", empty, empty);
    let digit = " 42".parse::<i32>().unwrap_err();
    assert_eq!(digit.kind(), &IntErrorKind::InvalidDigit);
    println!("{} {:?}", digit, digit);
    let positive = "128".parse::<i8>().unwrap_err();
    assert_eq!(positive.kind(), &IntErrorKind::PosOverflow);
    println!("{} {:?}", positive, positive);
    let negative = "-129".parse::<i8>().unwrap_err();
    assert_eq!(negative.kind(), &IntErrorKind::NegOverflow);
    println!("{} {:?}", negative, negative);
    assert!("-0".parse::<u8>().is_err());
    assert!("+".parse::<u128>().is_err());
    assert!("-".parse::<i128>().is_err());
    assert!("1_0".parse::<u64>().is_err());
    assert!("42 ".parse::<i32>().is_err());
    assert!("１２".parse::<i32>().is_err());
    assert!("1\0".parse::<u32>().is_err());
    assert!("170141183460469231731687303715884105728".parse::<i128>().is_err());
    assert!("-170141183460469231731687303715884105729".parse::<i128>().is_err());
    assert!("340282366920938463463374607431768211456".parse::<u128>().is_err());
}
"#,
                expect: Expect::Run(
                    "cannot parse integer from empty string ParseIntError { kind: Empty }\ninvalid digit found in string ParseIntError { kind: InvalidDigit }\nnumber too large to fit in target type ParseIntError { kind: PosOverflow }\nnumber too small to fit in target type ParseIntError { kind: NegOverflow }\n",
                ),
            },
            Case {
                name: "custom_from_str_and_core_path",
                source: r#"
use core::str::FromStr;
struct Value { n: i32 }
impl FromStr for Value {
    type Err = core::num::ParseIntError;
    fn from_str(s: &str) -> Result<Value, Self::Err> {
        match s.parse::<i32>() {
            Ok(n) => Ok(Value { n }),
            Err(e) => Err(e),
        }
    }
}
fn parse<T: FromStr>(s: &str) -> Result<T, T::Err> { s.parse() }
fn main() {
    let value = parse::<Value>("42").ok().unwrap();
    assert_eq!(value.n, 42);
    assert!(parse::<Value>("no").is_err());
}
"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_missing_from_str",
                source: "struct Value; fn unused() { let _v = \"1\".parse::<Value>(); } fn main() {}",
                expect: Expect::Invalid("E0277"),
            },
            Case {
                name: "unused_wrong_error_type",
                source: "fn unused() { let _v: Result<i32, u8> = \"1\".parse::<i32>(); } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "parser_static_error_does_not_borrow_input",
                source: r#"use std::str::FromStr;
struct Value;
impl FromStr for Value {
    type Err = &'static str;
    fn from_str(_s: &str) -> Result<Value, Self::Err> { Err("invalid") }
}
fn main() {
    let error;
    { let text = String::from("42"); error = text.parse::<Value>().err().unwrap(); }
    println!("{}", error);
}"#,
                expect: Expect::Run("invalid\n"),
            },
            Case {
                name: "unused_nested_static_cannot_borrow_input",
                source: "fn bad(s: &str) -> Result<i32, &'static str> { Err(s) } fn main() {}",
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "unused_parser_cannot_return_input_as_static_error",
                source: r#"use std::str::FromStr;
struct Value;
impl FromStr for Value {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Value, Self::Err> { Err(s) }
}
fn main() {}"#,
                expect: Expect::Invalid("lifetime may not live long enough"),
            },
            Case {
                name: "method_local_projection_preserves_exact_binder",
                source: r#"
trait Has { type Out; }
struct Item;
impl Has for Item { type Out = i32; }
trait Forward { fn forward<T: Has>(&self, value: T::Out) -> T::Out; }
struct Sender;
impl Forward for Sender {
    fn forward<T: Has>(&self, value: T::Out) -> T::Out { value }
}
fn main() { assert_eq!(Sender.forward::<Item>(42), 42); }
"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "unused_generic_projected_method_wrong_body",
                source: r#"
trait Has { type Out; }
trait Forward { fn forward<T: Has>(&self, value: T::Out) -> T::Out; }
struct Sender;
impl Forward for Sender {
    fn forward<T: Has>(&self, value: T::Out) -> T::Out { 42i32 }
}
fn main() {}
"#,
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn integer_from_str_boundary_matrix() {
    let mut inputs = vec![
        "".to_owned(),
        "+".to_owned(),
        "-".to_owned(),
        "+0".to_owned(),
        "-0".to_owned(),
        "00042".to_owned(),
        " 1".to_owned(),
        "1 ".to_owned(),
        "\t1".to_owned(),
        "1\n".to_owned(),
        "1_0".to_owned(),
        "0x10".to_owned(),
        "++1".to_owned(),
        "+-1".to_owned(),
        "١".to_owned(),
        "１２".to_owned(),
        "1\0".to_owned(),
        "127".to_owned(),
        "128".to_owned(),
        "-128".to_owned(),
        "-129".to_owned(),
        "255".to_owned(),
        "256".to_owned(),
        "32767".to_owned(),
        "32768".to_owned(),
        "-32768".to_owned(),
        "-32769".to_owned(),
        "65535".to_owned(),
        "65536".to_owned(),
        "2147483647".to_owned(),
        "2147483648".to_owned(),
        "-2147483648".to_owned(),
        "-2147483649".to_owned(),
        "4294967295".to_owned(),
        "4294967296".to_owned(),
        "9223372036854775807".to_owned(),
        "9223372036854775808".to_owned(),
        "-9223372036854775808".to_owned(),
        "-9223372036854775809".to_owned(),
        "18446744073709551615".to_owned(),
        "18446744073709551616".to_owned(),
        "170141183460469231731687303715884105727".to_owned(),
        "170141183460469231731687303715884105728".to_owned(),
        "-170141183460469231731687303715884105728".to_owned(),
        "-170141183460469231731687303715884105729".to_owned(),
        "340282366920938463463374607431768211455".to_owned(),
        "340282366920938463463374607431768211456".to_owned(),
        "99999999999999999999999999999999999999999999x".to_owned(),
        "-99999999999999999999999999999999999999999999x".to_owned(),
    ];
    inputs.push(format!("{}42", "0".repeat(1024)));
    let mut source = String::new();
    let mut calls = String::from("fn main() {");
    let mut expected = String::new();
    macro_rules! add_type {
        ($ty:ty) => {{
            let name = stringify!($ty);
            source.push_str(&format!("fn cases_{name}() {{ for text in {inputs:?} {{ println!(\"{{:?}}\", text.parse::<{name}>()); }} }}\n"));
            calls.push_str(&format!("cases_{name}();"));
            for input in &inputs {
                expected.push_str(&format!("{:?}\n", input.parse::<$ty>()));
            }
        }};
    }
    add_type!(i8);
    add_type!(i16);
    add_type!(i32);
    add_type!(i64);
    add_type!(i128);
    add_type!(isize);
    add_type!(u8);
    add_type!(u16);
    add_type!(u32);
    add_type!(u64);
    add_type!(u128);
    add_type!(usize);
    calls.push('}');
    source.push_str(&calls);
    parse_oracle(
        "parse-boundaries",
        &[closure_oracle::Case {
            name: "all_widths_syntax_and_overflow",
            source: Box::leak(source.into_boxed_str()),
            expect: closure_oracle::Expect::Run(Box::leak(expected.into_boxed_str())),
        }],
    );
}

#[test]
fn primitive_methods_match_stock() {
    assert!(
        std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .is_ok_and(|o| o.status.success()),
        "primitive differential tests require local stock rustc"
    );
    oracle::check(
        "primitive-methods",
        &[
            Case {
                name: "signed",
                source: r#"fn main() {
            assert_eq!((-7i32).abs(), 7); assert_eq!((-7i32).signum(), -1);
            assert_eq!(0i32.signum(), 0); assert_eq!(7i32.signum(), 1);
            assert!((-1i64).is_negative()); assert!(1i64.is_positive());
            assert_eq!(3i16.pow(4), 81); assert_eq!(127i8.pow(1), 127); assert_eq!(0i8.pow(0), 1);
            assert_eq!(7i32.min(3), 3); assert_eq!(7i32.max(3), 7);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "wrapping",
                source: r#"fn main() {
            assert_eq!(127i8.wrapping_add(1), -128); assert_eq!((-128i8).wrapping_sub(1), 127);
            assert_eq!(100i8.wrapping_mul(3), 44); assert_eq!((-128i8).wrapping_neg(), -128);
            assert_eq!(255u8.wrapping_add(2), 1); assert_eq!(0u8.wrapping_sub(1), 255);
            assert_eq!(255u8.wrapping_mul(255), 1);
            assert_eq!(127i8.saturating_add(1), 127); assert_eq!((-128i8).saturating_sub(1), -128);
            assert_eq!(255u8.saturating_add(1), 255); assert_eq!(0u8.saturating_sub(1), 0);
            assert_eq!(18446744073709551615u64.wrapping_mul(18446744073709551615u64), 1);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "bits",
                source: r#"fn main() {
            assert_eq!((-1i8).count_ones(), 8); assert_eq!(0u16.count_zeros(), 16);
            assert_eq!(0u128.trailing_zeros(), 128); assert_eq!(8u32.trailing_zeros(), 3);
            assert_eq!((-128i8).count_ones(), 1); assert!(128u8.is_power_of_two());
            assert!(!0u8.is_power_of_two()); assert!(!3u8.is_power_of_two());
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "characters",
                source: r#"fn main() {
            assert!('A'.is_ascii()); assert!('9'.is_ascii_digit()); assert!('f'.is_ascii_hexdigit());
            assert!('Z'.is_ascii_alphabetic()); assert!('3'.is_ascii_alphanumeric());
            assert!('A'.is_ascii_uppercase()); assert!('z'.is_ascii_lowercase());
            assert!('\t'.is_ascii_whitespace()); assert!(!'\u{b}'.is_ascii_whitespace());
            assert!('!'.is_ascii_punctuation()); assert!('~'.is_ascii_graphic());
            assert!('\u{7f}'.is_ascii_control()); assert!('\u{85}'.is_control());
            assert!('\u{2003}'.is_whitespace()); assert!(!'\u{200b}'.is_whitespace());
            assert!(!'é'.is_ascii()); assert_eq!('a'.to_ascii_uppercase(), 'A');
            assert_eq!('Z'.to_ascii_lowercase(), 'z'); assert_eq!('é'.to_ascii_uppercase(), 'é');
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "floats",
                source: r#"fn main() {
            assert_eq!(9.0f64.sqrt(), 3.0); assert_eq!((-4.0f32).abs(), 4.0);
            assert!(0.0f64.is_finite()); assert!((-0.0f32).is_sign_negative());
            assert!(0.0f64.is_sign_positive()); assert!(!(1.0f64).is_nan());
            let nan: f64 = 0.0f64 / 0.0f64; let inf: f64 = 1.0f64 / 0.0f64;
            assert!(nan.is_nan()); assert!(inf.is_infinite()); assert!(!inf.is_finite());
            assert_eq!(nan.min(2.0), 2.0); assert_eq!(2.0f64.min(nan), 2.0);
            assert_eq!(nan.max(2.0), 2.0); assert_eq!(2.0f64.max(nan), 2.0);
            assert_eq!(2.0f32.min(3.0), 2.0); assert_eq!(2.0f32.max(3.0), 3.0);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "autoderef",
                source: r#"fn main() {
            let n: i32 = -5; let r: &i32 = &n; let rr: &&i32 = &r;
            assert_eq!(rr.abs(), 5); let c: char = 'a'; let cr: &char = &c;
            assert!(cr.is_ascii_lowercase()); let mut f: f64 = 4.0;
            let fr: &mut f64 = &mut f; assert_eq!(fr.sqrt(), 2.0);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "evaluation_order",
                source: r#"fn main() {
            let mut n: i32 = 5;
            let result: i32 = n.min({ n = 1; 3 });
            assert_eq!(result, 3); assert_eq!(n, 1);
            let mut order: i32 = 0;
            let value: i32 = { order = order * 10 + 1; 7i32 }.wrapping_add({ order = order * 10 + 2; 3 });
            assert_eq!(value, 10); assert_eq!(order, 12);
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "wide_and_float_edges",
                source: r#"fn main() {
            assert_eq!((-170141183460469231731687303715884105727i128).abs(), 170141183460469231731687303715884105727i128);
            assert_eq!((-1i128).count_ones(), 128);
            assert_eq!(340282366920938463463374607431768211455u128.count_zeros(), 0);
            assert_eq!(2u128.pow(100), 1267650600228229401496703205376u128);
            assert_eq!((-9223372036854775808i64).saturating_add(-1), -9223372036854775808i64);
            assert_eq!(9223372036854775807i64.saturating_sub(-1), 9223372036854775807i64);
            assert_eq!(255u8.saturating_sub(1), 254);
            assert_eq!((-128i8).saturating_add(-1), -128);
            assert_eq!(127i8.saturating_sub(-1), 127);
            assert_eq!((-128i8).saturating_add(127), -1);
            assert_eq!(255u8.saturating_add(0), 255);
            assert_eq!((-0.0f64).min(0.0), 0.0); assert_eq!(0.0f64.min(-0.0), 0.0);
            assert_eq!((-0.0f64).max(0.0), 0.0); assert_eq!(0.0f64.max(-0.0), 0.0);
            assert_eq!((-0.0f32).min(0.0), 0.0); assert_eq!(0.0f32.min(-0.0), 0.0);
            assert_eq!((-0.0f32).max(0.0), 0.0); assert_eq!(0.0f32.max(-0.0), 0.0);
            let nan: f32 = 0.0f32 / 0.0f32;
            assert!(nan.min(nan).is_nan()); assert_eq!(2.0f32.min(nan), 2.0);
            assert_eq!(2.0f32.max(nan), 2.0); assert!((-1.0f32).sqrt().is_nan());
            assert!(!nan.is_finite()); assert!(!nan.is_infinite());
            assert!(!(-0.0f64).abs().is_sign_negative());
        }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "wrong_argument_type",
                source: "fn main() { let x: i32 = 2; x.wrapping_add(1u32); }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "wrong_pow_exponent",
                source: "fn main() { 2i32.pow(3i32); }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "extra_argument",
                source: "fn main() { 'a'.is_ascii(1); }",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "missing_argument",
                source: "fn main() { 1i32.min(); }",
                expect: Expect::Invalid("E0061"),
            },
            Case {
                name: "wrong_result",
                source: "fn main() { let x: bool = 1u8.count_ones(); }",
                expect: Expect::Invalid("E0308"),
            },
            Case {
                name: "unused_invalid",
                source: "fn unused() { 1.0f32.min(2.0f64); } fn main() {}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

#[test]
fn primitive_result_inference_matches_stock_at_both_worker_counts() {
    use std::{fs, process::Command};
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-primitive-inference-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    fn capture(
        command: &mut Command,
        context: &str,
        root: &std::path::Path,
    ) -> std::process::Output {
        use std::time::{Duration, Instant};
        let stdout = root.join("command.stdout");
        let stderr = root.join("command.stderr");
        command
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap());
        let mut child = command.spawn().unwrap();
        let start = Instant::now();
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if start.elapsed() > Duration::from_secs(30) {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("timed out {context}: {command:?}");
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        std::process::Output {
            status,
            stdout: fs::read(stdout).unwrap(),
            stderr: fs::read(stderr).unwrap(),
        }
    }
    let cases = [
        (
            "once",
            r#"
            fn next(calls: &mut u32) -> u128 {
                *calls += 1u32;
                18446744073709551616u128
            }
            fn main() {
                let mut calls = 0u32;
                let count = next(&mut calls).leading_zeros();
                let ones = next(&mut calls).count_ones();
                assert_eq!(calls, 2u32);
                assert_eq!(count, 63u32);
                assert_eq!(ones, 1u32);
                println!("ok");
            }"#,
            None,
        ),
        (
            "primitive_signatures",
            r#"
            fn main() {
                let absolute = (-4i32).abs();
                let character = 'a'.to_ascii_uppercase();
                let root = 9.0f64.sqrt();
                let power = 2i32.pow(3);
                let zeros = 8u32.trailing_zeros();
                let nested = 1u64.leading_zeros().count_ones();
                assert_eq!(absolute, 4i32);
                assert_eq!(character, 'A');
                assert_eq!(root, 3.0f64);
                assert_eq!(power, 8i32);
                assert_eq!(zeros, 3u32);
                assert_eq!(nested, 6u32);
                println!("ok");
            }"#,
            None,
        ),
        (
            "ordinary_method",
            r#"
            struct S { value: bool }
            impl S { fn leading_zeros(self) -> bool { self.value } }
            fn main() {
                let result = S { value: true }.leading_zeros();
                assert!(result);
                println!("ok");
            }"#,
            None,
        ),
        (
            "ordinary_scalar_trait_method",
            r#"
            trait UserCount { fn leading_zeros(self) -> bool; }
            impl UserCount for f32 { fn leading_zeros(self) -> bool { self == 1.0f32 } }
            fn main() {
                let result = 1.0f32.leading_zeros();
                assert!(result);
                println!("ok");
            }"#,
            None,
        ),
        (
            "wrong_result",
            "fn main(){let count: bool = 1u128.leading_zeros();}",
            Some("E0308"),
        ),
        (
            "wrong_argument",
            "fn main(){let result = 1i32.wrapping_add(1u32);}",
            Some("E0308"),
        ),
        (
            "wrong_arity",
            "fn main(){let count = 1u128.leading_zeros(1u32);}",
            Some("E0061"),
        ),
        (
            "wrong_type_arguments",
            "fn main(){let count = 1u128.leading_zeros::<u8>();}",
            Some("E0107"),
        ),
        (
            "noninteger",
            "fn main(){let count = 1.0f32.leading_zeros();}",
            Some("E0599"),
        ),
        (
            "unused_wrong_result",
            "fn unused(){let count: bool = 1u128.leading_zeros();} fn main(){}",
            Some("E0308"),
        ),
    ];
    std::thread::scope(|scope| {
        let mut jobs = Vec::new();
        for (name, source, invalid) in cases {
            let root = root.join(name);
            jobs.push(scope.spawn(move || {
                fs::create_dir(&root).unwrap();
                let source_path = root.join(format!("{name}.rs"));
                fs::write(&source_path, source).unwrap();
                for workers in [None, Some(1), Some(8)] {
                    for check in [true, false] {
                        let image = root.join(format!("{name}-{workers:?}-{check}"));
                        let mut command = if let Some(workers) = workers {
                            let mut command = Command::new(env!("CARGO_BIN_EXE_jr-rust"));
                            command.args(["--workers", &workers.to_string()]);
                            if check {
                                command.arg("--check");
                            }
                            command
                        } else {
                            let mut command = Command::new(stock_rustc::stock_rustc());
                            command.args(["--edition=2024", "--crate-name=oracle", "-Awarnings"]);
                            if check {
                                command.arg("--emit=metadata");
                            }
                            command
                        };
                        command.arg(&source_path);
                        if workers.is_none() || !check {
                            command.arg("-o").arg(&image);
                        }
                        let context = format!("{name} workers={workers:?} check={check}");
                        let output = capture(&mut command, &context, &root);
                        let error = String::from_utf8_lossy(&output.stderr);
                        if let Some(code) = invalid {
                            assert!(
                                !output.status.success(),
                                "{name} {workers:?}: accepted invalid source"
                            );
                            assert!(
                                output.status.code().is_some(),
                                "{name} {workers:?}: {error}"
                            );
                            assert!(!image.exists(), "{name}: invalid input published output");
                            if workers.is_none() {
                                assert!(error.contains(code), "{name}: {error}");
                            } else {
                                for forbidden in
                                    ["panicked at", "resource limit", "cannot progress"]
                                {
                                    assert!(!error.contains(forbidden), "{name}: {error}");
                                }
                            }
                        } else {
                            assert!(
                                output.status.success(),
                                "{name} {workers:?} check={check}: {error}"
                            );
                            if !check {
                                let ran = capture(&mut Command::new(&image), &context, &root);
                                assert!(ran.status.success(), "{name} {workers:?}: {ran:?}");
                                assert_eq!(ran.stdout, b"ok\n", "{name} {workers:?}");
                                assert!(ran.stderr.is_empty(), "{name} {workers:?}: {ran:?}");
                            } else if workers.is_some() {
                                assert!(!image.exists(), "CHECK published executable");
                            }
                        }
                    }
                }
            }));
        }
        for job in jobs {
            job.join().unwrap();
        }
    });
}

#[test]
fn checked_primitive_overflow_matches_stock() {
    use std::{fs, process::Command};
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-primitive-overflow-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(root.clone());
    for (name, source) in [
        (
            "abs_min",
            "fn main() { let n: i8 = -128; let result: i8 = n.abs(); assert_eq!(result, 0); }",
        ),
        (
            "pow_overflow",
            "fn main() { let n: i8 = 12; let result: i8 = n.pow(2); assert_eq!(result, 0); }",
        ),
    ] {
        let path = root.join("main.rs");
        fs::write(&path, source).unwrap();
        for (compiler, stock) in [("rustc", true), (env!("CARGO_BIN_EXE_jr-rust"), false)] {
            let image = root.join(format!("{name}-{}", if stock { "stock" } else { "native" }));
            let mut command = Command::new(compiler);
            if stock {
                command.env_remove("RUSTC_BOOTSTRAP").args([
                    "--edition=2024",
                    "-Cpanic=abort",
                    "-Coverflow-checks=yes",
                    "--crate-name=oracle",
                ]);
            }
            let output = command.arg(&path).arg("-o").arg(&image).output().unwrap();
            assert!(
                output.status.success(),
                "{name} {compiler}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let ran = Command::new(&image).output().unwrap();
            assert!(!ran.status.success(), "{name} {compiler} did not trap");
            assert!(
                !String::from_utf8_lossy(&ran.stderr).contains("assertion failed"),
                "{name} {compiler} reached the assertion instead of trapping overflow"
            );
        }
    }
}

#[test]
fn float_from_str_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "parse-float",
        &[
            Case {
                name: "correctly_rounded_edges_and_errors",
                source: r#"fn main(){
    for s in ["2.5","1.","0.1",".5","1e5","1E+5","-0","+3","inf","-Infinity","NaN","nan","1e400","1e-400","2.2250738585072014e-308","4.9e-324","2.4703282292062327e-324","2.4703282292062328e-324","0.30000000000000004","123456789012345678901234567890","9007199254740993","", ".", "e5", "1e", "+", " 1", "1 ", "0x10", "1_0", "1e+", "--1", "infinit", "-nan", "+inf", "INFINITY", "1e99999999999999999999", "-1e-99999999999999999999", "00000.000001e6", "1.7976931348623157e308", "1.7976931348623158e308", "1.7976931348623159e308", "2.2250738585072011e-308"] {
        let r: Result<f64, std::num::ParseFloatError> = s.parse::<f64>();
        match r {
            Ok(v) => { let b = v.to_bits(); println!("{:?} -> {}", s, b); }
            Err(e) => println!("{:?} -> err {} {:?}", s, e, e),
        }
    }
    for s in ["3.4028235e38","3.4028236e38","1e-46","1.4e-45","0.1","16777217","-nan"] {
        let v: f32 = s.parse().unwrap();
        let b = v.to_bits();
        println!("{:?} -> {}", s, b);
    }
}
"#,
                expect: Expect::Run(
                    r#""2.5" -> 4612811918334230528
"1." -> 4607182418800017408
"0.1" -> 4591870180066957722
".5" -> 4602678819172646912
"1e5" -> 4681608360884174848
"1E+5" -> 4681608360884174848
"-0" -> 9223372036854775808
"+3" -> 4613937818241073152
"inf" -> 9218868437227405312
"-Infinity" -> 18442240474082181120
"NaN" -> 9221120237041090560
"nan" -> 9221120237041090560
"1e400" -> 9218868437227405312
"1e-400" -> 0
"2.2250738585072014e-308" -> 4503599627370496
"4.9e-324" -> 1
"2.4703282292062327e-324" -> 0
"2.4703282292062328e-324" -> 1
"0.30000000000000004" -> 4599075939470750516
"123456789012345678901234567890" -> 5042042089369253694
"9007199254740993" -> 4845873199050653696
"" -> err cannot parse float from empty string ParseFloatError { kind: Empty }
"." -> err invalid float literal ParseFloatError { kind: Invalid }
"e5" -> err invalid float literal ParseFloatError { kind: Invalid }
"1e" -> err invalid float literal ParseFloatError { kind: Invalid }
"+" -> err invalid float literal ParseFloatError { kind: Invalid }
" 1" -> err invalid float literal ParseFloatError { kind: Invalid }
"1 " -> err invalid float literal ParseFloatError { kind: Invalid }
"0x10" -> err invalid float literal ParseFloatError { kind: Invalid }
"1_0" -> err invalid float literal ParseFloatError { kind: Invalid }
"1e+" -> err invalid float literal ParseFloatError { kind: Invalid }
"--1" -> err invalid float literal ParseFloatError { kind: Invalid }
"infinit" -> err invalid float literal ParseFloatError { kind: Invalid }
"-nan" -> 18444492273895866368
"+inf" -> 9218868437227405312
"INFINITY" -> 9218868437227405312
"1e99999999999999999999" -> 9218868437227405312
"-1e-99999999999999999999" -> 9223372036854775808
"00000.000001e6" -> 4607182418800017408
"1.7976931348623157e308" -> 9218868437227405311
"1.7976931348623158e308" -> 9218868437227405311
"1.7976931348623159e308" -> 9218868437227405312
"2.2250738585072011e-308" -> 4503599627370495
"3.4028235e38" -> 2139095039
"3.4028236e38" -> 2139095040
"1e-46" -> 0
"1.4e-45" -> 1
"0.1" -> 1036831949
"16777217" -> 1266679808
"-nan" -> 4290772992
"#,
                ),
            },
            Case {
                name: "corpus18_parse_with_map_err",
                source: "fn main(){let s=String::from(\"2.5\");let n:f64=s.parse().map_err(|_|format!(\"bad {}\",s)).unwrap();let b:Result<f64,String>=\"x\".parse::<f64>().map_err(|e|format!(\"{}\",e));println!(\"{} {:?}\",n,b);}",
                expect: Expect::Run("2.5 Err(\"invalid float literal\")\n"),
            },
            Case {
                name: "float_error_is_not_int_error",
                source: "fn bad(){let _e:std::num::ParseIntError=\"1.5\".parse::<f64>().unwrap_err();}fn main(){}",
                expect: Expect::Invalid("E0308"),
            },
        ],
    );
}

/// A closure parameter typed by a projection (`<T as FromStr>::Err`) whose
/// normal form is a source struct, with `T` known only from the annotation.
#[test]
fn closure_parameter_projection_to_struct_matches_stock() {
    use closure_oracle::{Case, Expect};
    parse_oracle(
        "closure-projection-struct",
        &[
            Case {
                name: "parse_map_err_question_mark",
                source: r#"fn f(s: String) -> Result<f64, String> {
    let n: f64 = s.parse().map_err(|_| format!("bad {}", s))?;
    Ok(n)
}
fn g(s: &str) -> Result<bool, u8> { let b: bool = s.parse().map_err(|_| 7u8)?; Ok(b) }
fn main() {
    println!("{:?} {:?}", f("2.5".to_string()), f("x".to_string()));
    println!("{:?} {:?}", g("true"), g("nope"));
    let r: Result<i32, u8> = "12".parse().map_err(|_| 0u8);
    println!("{:?}", r);
}"#,
                expect: Expect::Run("Ok(2.5) Err(\"bad x\")\nOk(true) Err(7)\nOk(12)\n"),
            },
            Case {
                name: "user_trait_struct_projection",
                source: r#"#[derive(Debug)]
struct MyErr;
trait Q: Sized { type Err; fn q(s: &str) -> Result<Self, Self::Err>; }
impl Q for i32 { type Err = MyErr; fn q(s: &str) -> Result<i32, MyErr> { if s.is_empty() { Err(MyErr) } else { Ok(7) } } }
fn get<T: Q>(s: &str) -> Result<T, T::Err> { T::q(s) }
fn main() {
    let a: Result<i32, u8> = get("x").map_err(|_| 0u8);
    let b: Result<i32, u8> = get("").map_err(|_e| 9u8);
    println!("{:?} {:?}", a, b);
}"#,
                expect: Expect::Run("Ok(7) Err(9)\n"),
            },
            Case {
                name: "closure_param_used_as_wrong_type",
                source: r#"fn bad(s: &str) -> Result<i32, u8> { let n: i32 = s.parse().map_err(|e: u8| e)?; Ok(n) } fn main() {}"#,
                expect: Expect::Invalid("E0271"),
            },
        ],
    );
}
