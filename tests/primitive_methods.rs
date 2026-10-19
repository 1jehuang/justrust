//! Stock rustc is an external execution and rejection oracle only.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

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
