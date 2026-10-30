//! Ordinary source trait lookup and lossless byte-to-char conversion.
#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use justrust_engine::compiler;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};

fn differential(name: &str, source: &str, valid: bool) {
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("char-from-{}-{name}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), source).unwrap();
    let stock =
        Command::new(std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into()))
            .args([
                "--edition=2024",
                "-Cpanic=abort",
                "-Awarnings",
                "--crate-name=oracle",
            ])
            .arg(root.join("main.rs"))
            .arg("-o")
            .arg(root.join("stock"))
            .output()
            .unwrap();
    assert_eq!(
        stock.status.success(),
        valid,
        "{name}: {}",
        String::from_utf8_lossy(&stock.stderr)
    );
    let expected = valid.then(|| Command::new(root.join("stock")).output().unwrap());
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.to_string(), workers, check);
            if !valid {
                assert!(result.is_err(), "{name}: accepted invalid source");
                continue;
            }
            let result = result
                .unwrap_or_else(|error| panic!("{name} workers={workers} check={check}: {error}"));
            if check {
                assert!(result.executable.is_none());
                continue;
            }
            let image = root.join(format!("native-{workers}"));
            fs::write(&image, result.executable.unwrap()).unwrap();
            fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
            let actual = Command::new(image).output().unwrap();
            let expected = expected.as_ref().unwrap();
            assert!(expected.status.success());
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
fn reflexive_from_preserves_generic_and_noncopy_values() {
    differential(
        "reflexive-values",
        r#"struct Item{value:i32}
fn same<T>(value:T)->T{T::from(value)}
fn into_same<T>(value:T)->T{value.into()}
fn main(){let a=Item{value:7};let b=Item{value:9};let n=11i32;println!("{} {} {} {} {}",same(a).value,into_same(b).value,i32::from(3i32),<bool as From<bool>>::from(true),*same(&n));}"#,
        true,
    );
    differential(
        "reflexive-field-single-evaluation",
        r#"struct Item{value:i32}fn make()->Item{println!("once");Item{value:7}}fn main(){println!("{}",Item::from(make()).value);}"#,
        true,
    );
}

#[test]
fn blanket_from_keeps_inherent_precedence_and_contextual_into_results() {
    differential(
        "inherent-and-contextual-conversion",
        r#"
        struct Mark { value: i32 }
        impl Mark { fn from(value:i32)->Self { Self { value:value+1 } } }
        impl From<i32> for Mark { fn from(value:i32)->Self { Self { value:value+2 } } }
        impl From<bool> for Mark { fn from(value:bool)->Self { Self { value:if value {3}else{0} } } }
        struct Record { name: String }
        struct Pick;
        trait Take<T> { fn take(value:T)->T; }
        impl Take<i32> for Pick { fn take(value:i32)->i32 { value+100 } }
        impl Take<bool> for Pick { fn take(value:bool)->bool { !value } }
        impl Pick { fn take<T>(value:T)->T { value } }
        fn main() {
            let records=vec![Record{name:"hello".into()},Record{name:"world".into()}];
            println!("{} {} {} {}",Mark::from(7i32).value,
                <Mark as From<i32>>::from(7i32).value,Pick::take(7i32),
                String::from("owned"));
            for record in records { println!("{}",record.name); }
        }
        "#,
        true,
    );
}

#[test]
fn reflexive_from_rejects_duplicate_impls_and_reused_moves() {
    for (name, source) in [
        (
            "reflexive-duplicate",
            "struct Item;impl From<Item> for Item{fn from(value:Item)->Item{value}}fn main(){}",
        ),
        (
            "reflexive-move",
            "struct Item{value:i32}fn main(){let a=Item{value:7};let b=Item::from(a);println!(\"{} {}\",a.value,b.value);}",
        ),
    ] {
        differential(name, source, false);
    }
}

#[test]
fn every_byte_is_the_same_unicode_scalar_through_canonical_from() {
    differential(
        "all-bytes",
        r#"
        fn convert<T: From<u8>>(value: u8) -> T { T::from(value) }
        fn main() {
            let mut n = 0u32;
            while n < 256u32 {
                let byte = n as u8;
                assert_eq!(char::from(byte) as u32, n);
                assert_eq!(<char as std::convert::From<u8>>::from(byte) as u32, n);
                assert_eq!(<char as core::convert::From<u8>>::from(byte) as u32, n);
                assert_eq!(convert::<char>(byte) as u32, n);
                n += 1u32;
            }
            assert_eq!(char::from(65), 'A');
            assert_eq!(char::from(0u8), '\0');
            assert_eq!(char::from(255u8), '\u{ff}');
        }
    "#,
        true,
    );
}

#[test]
fn foreign_from_and_char_names_keep_their_own_source_semantics() {
    differential(
        "foreign-names",
        r#"
        mod foreign {
            pub trait From<T> { fn from(value: T) -> Self; }
            pub struct Mark { pub value: u32 }
            impl From<u8> for Mark { fn from(value: u8) -> Self { Mark { value: value as u32 + 700u32 } } }
            pub fn run() -> u32 { <Mark as From<u8>>::from(3u8).value }
        }
        mod char { pub fn from(value: u8) -> u32 { value as u32 + 900u32 } }
        trait Make { fn make() -> Self; }
        impl Make for u16 { fn make() -> u16 { 321u16 } }
        type Scalar = char;
        type Identity<T> = T;
        mod empty { pub mod char {} pub fn value()->u32 { char::from(128u8) as u32 } }
        use std::convert::From as Convert;
        fn main() {
            assert_eq!(foreign::run(), 703u32); assert_eq!(char::from(2u8), 902u32);
            assert_eq!(u16::make(), 321u16);
            assert_eq!(Scalar::from(255u8) as u32, 255u32);
            assert_eq!(Identity::<char>::from(129u8) as u32, 129u32);
            assert_eq!(empty::value(), 128u32);
            assert_eq!(<Scalar as Convert<u8>>::from(128u8) as u32, 128u32);
        }
    "#,
        true,
    );
}

#[test]
fn validated_call_types_preserve_never_unsizing_and_generic_arguments() {
    differential(
        "call-shapes",
        r#"
        fn diverging_argument()->char { <char as From<u8>>::from(loop {}) }
        fn first(value:&[u8])->u8 { value[0] }
        fn identity<T:Copy>(value:T)->T { value }
        fn generic<T:Copy>(value:T)->T { identity::<T>(value) }
        fn main() {
            assert_eq!(char::from(first(b"AB")), 'A');
            assert_eq!(generic::<char>(char::from(255u8)), '\u{ff}');
        }
    "#,
        true,
    );
}

#[test]
fn invalid_conversions_and_unused_errors_are_rejected() {
    for (name, source) in [
        (
            "implicit-never",
            "fn bad()->char{char::from(loop{})} fn main(){}",
        ),
        ("u32", "fn main(){ let _: char = char::from(65u32); }"),
        ("signed", "fn main(){ let _: char = char::from(-1i8); }"),
        ("bool", "fn main(){ let _: char = char::from(true); }"),
        ("reference", "fn main(){ let _: char = char::from(&65u8); }"),
        ("result", "fn main(){ let _: u32 = char::from(65u8); }"),
        (
            "unused",
            "fn bad()->char { char::from(0xd800u32) } fn main(){}",
        ),
        (
            "shadowed-missing",
            "struct char; fn main(){ let _=char::from(1u8); }",
        ),
        (
            "qualified-missing",
            "mod outer{pub mod char{}} fn main(){let _=outer::char::from(1u8);}",
        ),
        (
            "ambiguous",
            "trait Other {fn from(x:u8)->Self;} impl Other for char{fn from(x:u8)->char{x as char}} fn main(){let _=char::from(1u8);}",
        ),
        (
            "foreign",
            "mod foreign { pub struct Mark; } fn main(){ let _ = <foreign::Mark as std::convert::From<u8>>::from(1u8); }",
        ),
    ] {
        differential(name, source, false);
    }
}

#[test]
fn lossless_integer_from_selects_the_argument_application() {
    differential(
        "integer-widening",
        r#"
        struct Pair { a: u16, b: u16 }
        impl Pair {
            fn checksum(&self) -> u32 { (u32::from(self.b) << 16) | u32::from(self.a) }
        }
        fn main() {
            let bytes: [u8; 3] = [255u8, 7u8, 128u8];
            let mut sum = 0u32;
            let mut index = 0usize;
            while index < 3usize { sum += u32::from(bytes[index]); index += 1usize; }
            assert_eq!(sum, 390u32);
            assert_eq!(u64::from(u32::from(65535u16)), 65535u64);
            assert_eq!(u64::from(7u16) + u64::from(9u8) + u64::from(true), 17u64);
            let (a, b, c): (i8, i16, i32) = (-5, -300, -70000);
            assert_eq!(i64::from(a) + i64::from(b) + i64::from(c), -70305i64);
            assert_eq!(u128::from(u64::MAX), 18446744073709551615u128);
            assert_eq!(usize::from(65535u16), 65535usize);
            assert_eq!(isize::from(b), -300isize);
            assert_eq!(i32::from(255u8), 255i32);
            assert_eq!(u8::from(false), 0u8);
            assert_eq!(<u32 as From<u16>>::from(3u16), 3u32);
            let pair = Pair { a: 1u16, b: 2u16 };
            assert_eq!(pair.checksum(), 131073u32);
            println!("{}", u32::from(pair.a) + u32::from(255u8));
        }
    "#,
        true,
    );
}

#[test]
fn lossy_or_missing_integer_from_is_rejected() {
    for (name, source) in [
        ("narrowing", "fn main(){ let _ = u16::from(70000u32); }"),
        ("u32-usize", "fn main(){ let _ = usize::from(1u32); }"),
        ("signed-unsigned", "fn main(){ let _ = u16::from(-1i8); }"),
        (
            "unsigned-same-signed",
            "fn main(){ let _ = i32::from(1u32); }",
        ),
        ("float", "fn main(){ let _ = u32::from(1.0f32); }"),
        (
            "unused-narrowing",
            "fn bad()->u8 { u8::from(1u16) } fn main(){}",
        ),
    ] {
        differential(name, source, false);
    }
}
