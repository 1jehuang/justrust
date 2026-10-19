//! Ordinary Vec wrappers around the canonical raw-slice constructors.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

const SOURCE: &str = r#"
struct Token { value: u64 }
fn shared_len<T>(values: &Vec<T>) -> usize {
    let slice = values.as_slice();
    slice.len()
}
fn unique_len<T>(values: &mut Vec<T>) -> usize {
    let slice = values.as_mut_slice();
    slice.len()
}
fn main() {
    let mut empty: Vec<u128> = Vec::new();
    let shared = empty.as_slice();
    let empty_len = shared.len();
    let address = empty.as_ptr() as usize;
    let nonnull = address != 0usize;
    let aligned = address % std::mem::align_of::<u128>() == 0usize;
    println!("{} {} {}", empty_len, nonnull, aligned);
    let unique = empty.as_mut_slice();
    let empty_unique_len = unique.len();
    println!("{}", empty_unique_len);
    let mut bytes: Vec<u8> = Vec::new();
    bytes.push(3u8); bytes.push(7u8);
    { let slice = bytes.as_slice(); let len = slice.len(); let a = slice[0usize]; let b = slice[1usize]; println!("{} {} {}", len, a, b); }
    { let slice = bytes.as_mut_slice(); slice[0usize] = 11u8; slice[1usize] += 2u8; }
    let byte_a = bytes[0usize]; let byte_b = bytes[1usize];
    println!("{} {}", byte_a, byte_b);
    let mut tokens: Vec<Token> = Vec::new();
    let empty_shared_len = shared_len(&tokens);
    let empty_mut_len = unique_len(&mut tokens);
    println!("{} {}", empty_shared_len, empty_mut_len);
    tokens.push(Token { value: 19u64 }); tokens.push(Token { value: 23u64 });
    let full_shared_len = shared_len(&tokens);
    let full_mut_len = unique_len(&mut tokens);
    println!("{} {}", full_shared_len, full_mut_len);
    { let slice = tokens.as_slice(); let first = &slice[0usize]; let value = first.value; println!("{}", value); }
    { let slice = tokens.as_mut_slice(); let second = &mut slice[1usize]; second.value = 29u64; }
    let final_token = &tokens[1usize]; let final_value = final_token.value;
    println!("{}", final_value);
}
"#;

fn root(tag: &str) -> std::path::PathBuf {
    let path = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-vec-slices-{}-{tag}", std::process::id()));
    fs::create_dir_all(&path).unwrap();
    path
}
fn stock(source: &str, root: &std::path::Path) -> std::process::Output {
    fs::write(root.join("main.rs"), source).unwrap();
    let rustc = std::env::var_os("JUSTRUST_REAL_RUSTC").unwrap_or_else(|| "rustc".into());
    Command::new(rustc)
        .args(["--edition", "2024", "-C", "panic=abort"])
        .arg(root.join("main.rs"))
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap()
}

#[test]
fn vec_slices_empty_nonempty_noncopy_and_mutation_match_stock() {
    let root = root("native");
    let oracle = stock(SOURCE, &root);
    assert!(
        oracle.status.success(),
        "{}",
        String::from_utf8_lossy(&oracle.stderr)
    );
    let expected = Command::new(root.join("stock")).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        let checked = compiler::compile(SOURCE.into(), workers, true)
            .unwrap_or_else(|error| panic!("workers={workers} CHECK: {error}"));
        assert!(checked.executable.is_none());
        assert!(checked.report.capture_complete);
        assert!(!checked.report.published_kind(103));
        assert!(checked.analyses.iter().all(|analysis| analysis.passed()));
        let built = compiler::compile(SOURCE.into(), workers, false)
            .unwrap_or_else(|error| panic!("workers={workers} BUILD: {error}"));
        let binary = root.join(format!("native-{workers}"));
        fs::write(&binary, built.executable.unwrap()).unwrap();
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o700)).unwrap();
        let actual = Command::new(binary).output().unwrap();
        assert!(actual.status.success(), "{actual:?}");
        assert_eq!(actual.stdout, expected.stdout, "workers={workers}");
    }
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn vec_slices_aliases_and_local_escape_are_rejected_without_image() {
    for (index, source) in [
        "fn main(){let mut v:Vec<u64>=Vec::new();v.push(1u64);let a=v.as_mut_slice();let b=v.as_mut_slice();a[0usize]=2u64;println!(\"{}\",b[0usize]);}",
        "fn main(){let mut v:Vec<u64>=Vec::new();v.push(1u64);let a=v.as_slice();let b=v.as_mut_slice();b[0usize]=2u64;println!(\"{}\",a[0usize]);}",
        "fn bad<'a>()->&'a[u64]{let mut v:Vec<u64>=Vec::new();v.push(1u64);v.as_slice()}fn main(){}",
    ].into_iter().enumerate() {
        let root = root(&format!("negative-{index}"));
        let oracle = stock(source, &root);
        assert!(!oracle.status.success(), "invalid stock control {index} accepted");
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(compiler::compile(source.into(),workers,check).is_err(),
                    "alias/escape {index} accepted workers={workers} check={check}");
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
