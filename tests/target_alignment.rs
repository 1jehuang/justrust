//! Genuine target alignment is distinct from the compiler's private storage stride.
use justrust_engine::compiler;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
#[path = "support/stock_rustc.rs"]
mod stock_rustc;

fn stock(source: &str, tag: &str) -> (std::path::PathBuf, std::process::Output) {
    let root = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join(format!("jr-target-alignment-{}-{tag}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("main.rs"), source).unwrap();
    let output = Command::new(stock_rustc::stock_rustc())
        .env_remove("RUSTC_BOOTSTRAP")
        .args(["--edition=2024", "-Cpanic=abort"])
        .arg(root.join("main.rs"))
        .arg("-o")
        .arg(root.join("stock"))
        .output()
        .unwrap();
    (root, output)
}

#[test]
fn target_alignment_scalars_arrays_structs_and_generic_instances() {
    let source = r#"
struct Narrow { first: u8, second: bool }
struct Wide { small: u8, wide: u128 }
struct Nested { inner: Wide, tail: [u16; 3] }
fn alignment<T>() -> usize { std::mem::align_of::<T>() }
fn main() {
    println!("{} {} {} {} {}", alignment::<()>(), alignment::<bool>(), alignment::<char>(), alignment::<f32>(), alignment::<f64>());
    println!("{} {} {} {} {}", alignment::<u8>(), alignment::<u16>(), alignment::<u32>(), alignment::<u64>(), alignment::<u128>());
    println!("{} {} {} {} {}", alignment::<i8>(), alignment::<i16>(), alignment::<i32>(), alignment::<i64>(), alignment::<i128>());
    println!("{} {} {} {}", alignment::<usize>(), alignment::<isize>(), alignment::<*const u8>(), alignment::<*mut u128>());
    println!("{} {} {} {}", alignment::<[u8; 0]>(), alignment::<[u16; 7]>(), alignment::<[u128; 3]>(), alignment::<[[u32; 2]; 4]>());
    println!("{} {} {}", alignment::<Narrow>(), alignment::<Wide>(), alignment::<Nested>());
}
"#;
    let (root, compiled) = stock(source, "native");
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let expected = Command::new(root.join("stock")).output().unwrap();
    assert!(expected.status.success());
    for workers in [1, 8] {
        compiler::compile(source.into(), workers, true).unwrap();
        let built = compiler::compile(source.into(), workers, false).unwrap();
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
fn target_alignment_private_operation_rejects_foreign_source_owners() {
    for source in [
        "fn main(){let value=__justrust_align_of!(u8);}",
        "mod mem { pub fn align_of<T>()->usize { __justrust_align_of!(T) } } fn main(){let value=mem::align_of::<u128>();}",
        "mod __justrust_std {pub mod mem {pub fn align_of<T>()->usize {__justrust_align_of!(T)}}} fn main(){let value=__justrust_std::mem::align_of::<u8>();}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("invalid or unsupported alignment must be rejected")
                    .to_string();
                assert!(
                    error.contains("private target alignment")
                        || error.contains("canonical source")
                        || error.contains("duplicate native prelude module"),
                    "{error}"
                );
            }
        }
    }
}

#[test]
fn target_alignment_unknown_layouts_fail_explicitly_instead_of_using_private_storage() {
    for (tag, source) in [
        (
            "enum",
            "enum E {A, B(u128)} fn main(){println!(\"{}\",std::mem::align_of::<E>());}",
        ),
        (
            "unsized-pointer",
            "fn main(){println!(\"{}\",std::mem::align_of::<*const str>());}",
        ),
        (
            "closure",
            "fn alignment<T>(value:T)->usize{std::mem::align_of::<T>()} fn main(){let closure=||1i32; println!(\"{}\",alignment(closure));}",
        ),
        (
            "overaligned",
            "#[repr(align(64))] struct Aligned {field:u8} fn main(){println!(\"{}\",std::mem::align_of::<Aligned>());}",
        ),
    ] {
        let (root, compiled) = stock(source, tag);
        assert!(
            compiled.status.success(),
            "{tag}: {}",
            String::from_utf8_lossy(&compiled.stderr)
        );
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile(source.into(), workers, check)
                    .err()
                    .expect("invalid or unsupported alignment must be rejected")
                    .to_string();
                if tag == "overaligned" {
                    // The attribute parser rejects this spelling before a layout query
                    // can be issued. This is an ingress boundary, not layout support.
                    assert!(
                        error.contains("expected attribute identifier"),
                        "{tag}: {error}"
                    );
                } else {
                    assert!(
                        error.to_lowercase().contains("unsupported"),
                        "{tag}: workers={workers}, check={check}: {error}"
                    );
                }
            }
        }
        fs::remove_dir_all(root).unwrap();
    }
}
