//! Stock-differential source acceptance for Rust raw text and byte literal values.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn differential(name: &str, source: &str, valid: bool) {
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("raw-byte-{}-{name}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let input = root.join("main.rs");
    fs::write(&input, source).unwrap();
    let rustc = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    let stock = Command::new(rustc)
        .args([
            "--edition=2024",
            "-Cpanic=abort",
            "--crate-name=oracle",
            "-Awarnings",
        ])
        .arg(&input)
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    assert_eq!(
        stock.status.success(),
        valid,
        "{name}: stock {}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let expected = valid.then(|| Command::new(root.join("stock")).output().unwrap());
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.to_string(), workers, check);
            if !valid {
                assert!(
                    result.is_err(),
                    "{name}: accepted invalid source workers={workers} check={check}"
                );
                continue;
            }
            let result =
                result.unwrap_or_else(|e| panic!("{name} workers={workers} check={check}: {e}"));
            if check {
                assert!(result.executable.is_none());
                continue;
            }
            let image = root.join(format!("native-{workers}"));
            fs::write(&image, result.executable.unwrap()).unwrap();
            fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
            let actual = Command::new(image).output().unwrap();
            let expected = expected.as_ref().unwrap();
            assert!(
                expected.status.success(),
                "stock execution {name}: {expected:?}"
            );
            assert_eq!(
                actual.status.code(),
                expected.status.code(),
                "{name}: {actual:?}"
            );
            assert_eq!(actual.stdout, expected.stdout, "{name}");
            assert_eq!(actual.stderr, expected.stderr, "{name}");
        }
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn raw_text_delimiters_unicode_crlf_and_macro_values() {
    differential(
        "raw-text",
        r####"fn text() -> &'static str { r##"a"#b\n水"## }
        fn main() {
            assert_eq!(text(), "a\"#b\\n水");
            assert_eq!(r"", ""); assert_eq!(r#"a\0b"#, "a\\0b");
            macro_rules! value { ($x:literal) => { $x }; }
            macro_rules! forward { ($x:literal) => { value!($x) }; }
            assert_eq!(forward!(r#"raw\n"#), "raw\\n");
            println!(r#"raw text ok"#);
        }"####,
        true,
    );
    differential(
        "crlf",
        "fn main(){assert_eq!(r\"a\r\nb\",\"a\\nb\");assert_eq!(br\"a\r\nb\"[1],10u8);}",
        true,
    );
}

#[test]
fn byte_strings_have_exact_static_array_type_and_slice_coercion() {
    differential(
        "static-bytes",
        r####"const ALPHABET: &[u8] = b"0123456789abcdfghijklmnpqrsvwxyz";
        const PAIR: &[u8; 2] = br"AB";
        fn array() -> &'static [u8; 4] { b"a\xFF\0z" }
        fn slice() -> &'static [u8; 3] { b"xyz" }
        fn read(x: &[u8]) -> u8 { x[1] }
        fn main() {
            let x: &[u8; 4] = array();
            assert_eq!(x[0], b'a'); assert_eq!(x[1], 255u8); assert_eq!(x[2], 0u8);
            assert_eq!(read(b"AB"), b'B'); let returned: &[u8] = slice(); assert_eq!(returned[2], b'z');
            assert_eq!(ALPHABET[31], b'z'); assert_eq!(PAIR[0], b'A');
            let empty: &[u8; 0] = b""; let s: &[u8] = empty; assert_eq!(s.len(), 0usize);
            let raw: &[u8; 6] = br##"a"#\nz"##;
            assert_eq!(raw[1], b'"'); assert_eq!(raw[3], b'\\'); assert_eq!(raw[4], b'n');
            println!("static bytes ok");
        }"####,
        true,
    );
}

#[test]
fn escapes_continuations_embedded_nul_and_macro_forwarding() {
    differential(
        "escapes",
        r####"fn main() {
        let bytes: &[u8; 10] = b"\n\r\t\0\\\'\"\x00\x80\xff";
        assert_eq!(bytes[0], 10u8); assert_eq!(bytes[3], 0u8); assert_eq!(bytes[7], 0u8);
        assert_eq!(bytes[8], 128u8); assert_eq!(bytes[9], 255u8);
        let continued: &[u8; 2] = b"a\
            b"; assert_eq!(continued[1], b'b');
        macro_rules! literal { ($x:literal) => { $x }; }
        macro_rules! forward { ($x:literal) => { literal!($x) }; }
        let a: &[u8; 2] = forward!(b"\0\xff"); assert_eq!(a[1], 255u8);
        let b: &[u8; 3] = forward!(br#"\n!"#); assert_eq!(b[0], b'\\');
        assert_eq!(forward!(b'\xff'), 255u8);
    }"####,
        true,
    );
    differential(
        "actual-nul",
        "fn main(){let a: &[u8;3]=b\"a\0b\";let b: &[u8;3]=br\"a\0b\";assert_eq!(a[1],0u8);assert_eq!(b[1],0u8);}",
        true,
    );
}

#[test]
fn invalid_lexical_forms_and_unused_type_errors_are_rejected() {
    for (name, source) in [
        ("nonascii", "fn main(){let _x=b\"é\";}"),
        ("raw-nonascii", "fn main(){let _x=br\"水\";}"),
        ("unicode-escape", r#"fn main(){let _x=b"\u{41}";}"#),
        ("unknown-escape", r#"fn main(){let _x=b"\q";}"#),
        ("short-hex", r#"fn main(){let _x=b"\xF";}"#),
        ("suffix", r#"fn main(){let _x=b"a"u8;}"#),
        ("raw-suffix", r##"fn main(){let _x=r#"a"#foo;}"##),
        ("unclosed-raw", "fn main(){let _x=br##\"a\"#;}"),
        ("bare-cr", "fn main(){let _x=br\"a\rb\";}"),
        (
            "wrong-length",
            r#"fn unused(){let _x:&[u8;3]=b"ab";}fn main(){}"#,
        ),
        ("wrong-str", r#"fn main(){let _x:&str=b"ab";}"#),
        ("wrong-bytes", r#"fn main(){let _x:&[u8]=r"ab";}"#),
        ("mutability", r#"fn main(){let _x:&mut [u8;2]=b"ab";}"#),
        (
            "local-static-frame",
            "fn main(){let local=[1u8,2];let _x:&'static[u8;2]=&local;}",
        ),
        (
            "mixed-static-frame-escape",
            "fn leak()->&'static [u8;2]{let _literal=b\"ok\";let x=[1u8,2];&x}fn main(){}",
        ),
        (
            "frame-escape",
            "fn leak()->&'static [u8;2]{let x=[1u8,2];&x}fn main(){}",
        ),
    ] {
        differential(name, source, false);
    }
    let hashes = "#".repeat(256);
    differential(
        "too-many-hashes",
        &format!("fn main(){{let _x=r{hashes}\"a\"{hashes};}}"),
        false,
    );
}

#[test]
fn c_strings_remain_explicitly_unsupported() {
    for source in [
        r#"fn main(){let _x=c"abc";}"#,
        r##"fn main(){let _x=cr#"abc"#;}"##,
    ] {
        let error = compiler::compile(source.into(), 1, true)
            .err()
            .unwrap()
            .to_string();
        assert!(error.contains("unsupported"), "{error}");
    }
}
