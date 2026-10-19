//! Byte literals remain exact rigid u8 values in the existing integer graph.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[path = "support/oracle.rs"]
mod oracle;
use oracle::{Case, Expect};

fn require_stock() {
    assert!(
        std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .is_ok_and(|o| o.status.success()),
        "byte differential tests require local stock rustc"
    );
}

#[test]
fn byte_values_types_patterns_and_macros_match_stock() {
    require_stock();
    oracle::check(
        "byte-literals",
        &[
            Case {
                name: "ascii_and_escapes",
                source: r#"fn byte(x: u8) -> u8 { x }
            fn main() {
                assert_eq!(byte(b'a'), 97u8); assert_eq!(b'\n', 10u8);
                assert_eq!(b'\r', 13u8); assert_eq!(b'\t', 9u8); assert_eq!(b'\0', 0u8);
                assert_eq!(b'\\', 92u8); assert_eq!(b'\'', 39u8); assert_eq!(b'\"', 34u8);
                assert_eq!(b'"', 34u8); assert_eq!(b'\x7f', 127u8);
                assert_eq!(b'\x80', 128u8); assert_eq!(b'\xFF', 255u8);
                assert_eq!(b'A' + 1, b'B'); assert_eq!(b'\xff'.wrapping_add(1), b'\0');
                assert_eq!(b'\xff' as char, 'ÿ'); println!("byte oracle ok");
            }"#,
                expect: Expect::Run("byte oracle ok\n"),
            },
            Case {
                name: "patterns_arrays_and_borrows",
                source: r#"fn read(x: &u8) -> u8 { *x }
            fn category(x: u8) -> u8 { match x { b'0'..=b'9' => 1, b'A' => 2, _ => 0 } }
            fn main() {
                let a = [b'\0', b'a', b'\xff']; assert_eq!(a[1], 97u8);
                let r = [b'x'; 3]; assert_eq!(r[2], b'x');
                assert_eq!(category(b'7'), 1); assert_eq!(category(b'A'), 2); assert_eq!(category(b'z'), 0);
                assert_eq!(read(&b'\xff'), 255u8);
                let mut n: u8 = 0; for _ in b'a'..=b'c' { n += 1; } assert_eq!(n, 3);
            }"#,
                expect: Expect::Run(""),
            },
            Case {
                name: "macro_literal_and_tt_forwarding",
                source: r#"fn main() { macro_rules! value { () => { b'\xff' }; }
            macro_rules! literal { ($x:literal) => { $x }; }
            macro_rules! forwarded { ($x:literal) => { literal!($x) }; }
            macro_rules! expression { ($x:tt) => { $x }; }
            let a: u8 = value!(); assert_eq!(a, 255u8);
                assert_eq!(forwarded!(b'\n'), 10u8); assert_eq!(expression!(b'A'), 65u8);
            }"#,
                expect: Expect::Run(""),
            },
        ],
    );
}

#[test]
fn every_hex_byte_matches_stock() {
    require_stock();
    // Separate functions keep each existing per-task graph budget proportional
    // to the input rather than constructing one giant assertion block.
    let mut source = String::new();
    for value in 0..=255u16 {
        source.push_str(&format!(
            "fn byte_{value}() {{ let x: u8 = b'\\x{value:02X}'; assert_eq!(x, {value}u8); }}\n"
        ));
    }
    source.push_str("fn main() {");
    for value in 0..=255u16 {
        source.push_str(&format!("byte_{value}();"));
    }
    source.push('}');
    oracle::check(
        "all-hex-bytes",
        &[Case {
            name: "all_256_hex_bytes",
            source: Box::leak(source.into_boxed_str()),
            expect: Expect::Run(""),
        }],
    );
}

#[test]
fn invalid_byte_literals_match_stock() {
    require_stock();
    let cases = [
        ("empty", "fn main() { let x = b''; }", ""),
        ("multiple", "fn main() { let x = b'ab'; }", ""),
        ("nonascii_latin", "fn main() { let x = b'é'; }", ""),
        ("nonascii_cjk", "fn main() { let x = b'水'; }", ""),
        ("nonascii_emoji", "fn main() { let x = b'🦀'; }", ""),
        ("unicode_escape", r"fn main() { let x = b'\u{41}'; }", ""),
        ("short_hex", r"fn main() { let x = b'\xF'; }", ""),
        ("nonhex", r"fn main() { let x = b'\xGG'; }", ""),
        ("extra_hex", r"fn main() { let x = b'\xFFF'; }", ""),
        (
            "unicode_hex_digits",
            r"fn main() { let x = b'\x１２'; }",
            "",
        ),
        ("unknown_escape", r"fn main() { let x = b'\q'; }", ""),
        ("raw_newline", "fn main() { let x = b'\n'; }", ""),
        ("raw_tab", "fn main() { let x = b'\t'; }", ""),
        ("raw_cr", "fn main() { let x = b'\r'; }", ""),
        ("suffix", "fn main() { let x = b'a'u8; }", ""),
        (
            "wrong_integer_type",
            "fn main() { let x: i32 = b'a'; }",
            "E0308",
        ),
        (
            "wrong_char_type",
            "fn main() { let x: char = b'a'; }",
            "E0308",
        ),
        (
            "wrong_pattern_type",
            "fn main() { match 'a' { b'a' => (), _ => () } }",
            "E0308",
        ),
        (
            "wrong_array_type",
            "fn main() { let x = [b'a', 1u16]; }",
            "E0308",
        ),
        ("negated_byte", "fn main() { let x = -b'a'; }", "E0600"),
        (
            "unused_invalid",
            "fn unused() { let x: i32 = b'a'; } fn main() {}",
            "E0308",
        ),
    ];
    let cases: Vec<_> = cases
        .into_iter()
        .map(|(name, source, diagnostic)| Case {
            name,
            source,
            expect: Expect::Invalid(diagnostic),
        })
        .collect();
    oracle::check("invalid-byte-literals", &cases);
}
