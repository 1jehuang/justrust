//! Genuine item-macro metadata retains expansion identity and normal body checks.
use justrust_engine::compiler;
#[path = "support/oracle.rs"]
mod oracle;

fn run_worker_image(image: Vec<u8>, workers: usize, tag: &str, expected: &[u8]) {
    use std::os::unix::fs::PermissionsExt;
    let base = std::env::var_os("JCODE_SCRATCH_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = base.join(format!(
        "jr-generated-{tag}-{}-{workers}-{nonce}",
        std::process::id()
    ));
    std::fs::write(&path, image).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    let result = std::process::Command::new(&path).output();
    std::fs::remove_file(&path).unwrap();
    let result = result.unwrap();
    assert!(result.status.success(), "workers={workers}: {result:?}");
    assert_eq!(result.stdout, expected, "workers={workers}");
    assert!(result.stderr.is_empty(), "workers={workers}: {result:?}");
}

const SOURCE: &str = r#"
macro_rules! item {
    ($text:literal) => {
        #[doc = concat!($text, stringify!(Unit))]
        struct Unit;
        impl Unit {
            #[doc = concat!("value of ", stringify!(Unit))]
            fn value() -> u8 { 7 }
        }
    }
}
item!("An expanded ");
fn main() { println!("{}", Unit::value()); }
"#;

const COPY_SOURCE: &str = r#"
macro_rules! item { () => { #[doc = "original generated unit"] #[derive(Copy)] struct Unit; } }
item!();
impl Clone for Unit { fn clone(&self) -> Self { Unit } }
fn duplicate<T:Copy>(value:T) -> (T,T) { (value,value) }
fn main() { let pair = duplicate(Unit); let again = pair.0; let original = pair.0; println!("7"); }
"#;

const COMBINED_SOURCE: &str = r#"
macro_rules! item {
    ($t:ident, $text:literal) => {
        #[doc = concat!("A unit representing one ", $text, ".")]
        #[derive(Debug, Clone, Copy)]
        pub struct $t;
    }
}
item!(Unit, "unit");
fn duplicate<T: Copy>(value:T) -> (T,T) { (value,value) }
fn main() {
    let original = Unit;
    let pair = duplicate(original);
    let cloned = original.clone();
    println!("{:?} {:?} {:?}", pair.0, pair.1, cloned);
}
"#;

#[test]
fn generated_combined_unit_derives_use_real_source_members() {
    oracle::check(
        "generated-combined-unit-derive",
        &[oracle::Case {
            name: "original_combined_metadata_and_copy_clone_debug",
            source: COMBINED_SOURCE,
            expect: oracle::Expect::Run("Unit Unit Unit\n"),
        }],
    );
    for workers in [1, 8] {
        let check = compiler::compile_observed(COMBINED_SOURCE.into(), workers, true)
            .unwrap_or_else(|error| panic!("combined CHECK workers={workers}: {error:?}"));
        assert!(check.executable.is_none());
        let image = compiler::compile_observed(COMBINED_SOURCE.into(), workers, false)
            .unwrap_or_else(|error| panic!("combined BUILD workers={workers}: {error:?}"))
            .executable
            .unwrap();
        run_worker_image(image, workers, "combined-unit", b"Unit Unit Unit\n");
    }
}

#[test]
fn generated_unit_copy_is_a_real_checked_implementation() {
    oracle::check(
        "generated-copy-metadata",
        &[oracle::Case {
            name: "canonical_copy_derive_and_real_clone_impl",
            source: COPY_SOURCE,
            expect: oracle::Expect::Run("7\n"),
        }],
    );
    for workers in [1, 8] {
        assert!(
            compiler::compile_observed(COPY_SOURCE.into(), workers, true)
                .unwrap()
                .executable
                .is_none()
        );
        let image = compiler::compile_observed(COPY_SOURCE.into(), workers, false)
            .unwrap()
            .executable
            .unwrap();
        run_worker_image(image, workers, "copy", b"7\n");
    }
}

#[test]
fn generated_copy_preserves_unused_obligations_and_unsupported_derive_boundaries() {
    for source in [
        "macro_rules! item{()=>{#[derive(Copy)]struct Unit;}}item!();fn main(){}",
        "macro_rules! item{()=>{#[derive(Copy,Copy)]struct Unit;}}item!();impl Clone for Unit{fn clone(&self)->Self{Unit}}fn main(){}",
        "macro_rules! item{()=>{#[derive(Copy)]struct Unit;}}item!();impl Clone for Unit{fn clone(&self)->Self{Unit}}impl Drop for Unit{fn drop(&mut self){}}fn main(){}",
        "macro_rules! item{()=>{#[derive(Copy)]struct Unit{field:String}}}item!();fn main(){}",
        "macro_rules! item{()=>{#[derive(Copy)]struct Unit<T>(T);}}item!();fn main(){}",
        "macro_rules! Copy{()=>{}}macro_rules! item{()=>{#[derive(Copy)]struct Unit;}}item!();fn main(){}",
        "macro_rules! item{()=>{#[derive(Clone,Clone,Copy)]struct Unit;}}item!();fn main(){}",
        "macro_rules! Debug{()=>{}}macro_rules! item{()=>{#[derive(Debug)]struct Unit;}}item!();fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(
                    compiler::compile_observed(source.into(), workers, check).is_err(),
                    "accepted {source}"
                );
            }
        }
    }
}

#[test]
fn generated_docs_with_resolved_builtin_values_match_stock() {
    oracle::check(
        "generated-doc-metadata",
        &[oracle::Case {
            name: "original_macro_item_and_member_doc_values",
            source: SOURCE,
            expect: oracle::Expect::Run("7\n"),
        }],
    );
    for workers in [1, 8] {
        assert!(
            compiler::compile_observed(SOURCE.into(), workers, true)
                .unwrap()
                .executable
                .is_none()
        );
        let image = compiler::compile_observed(SOURCE.into(), workers, false)
            .unwrap()
            .executable
            .unwrap();
        run_worker_image(image, workers, "docs", b"7\n");
    }
}

#[test]
fn generated_metadata_does_not_suppress_invalid_unused_items_or_activate_cfg() {
    for source in [
        "macro_rules! item{()=>{#[doc=\"body\"]fn unused()->u8{true}}}item!();fn main(){}",
        "macro_rules! item{()=>{#[doc=\"type\"]struct S{field:Missing}}}item!();fn main(){}",
        "macro_rules! item{()=>{#[cfg(any())]fn unused()->u8{true}}}item!();fn main(){}",
        "macro_rules! item{()=>{#[doc=concat!(\"x\",)]}}item!();fn main(){}",
        "macro_rules! concat{($($x:tt)*)=>{\"shadowed\"}}macro_rules! item{()=>{#[doc=concat!(\"x\")]struct S;}}item!();fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                assert!(
                    compiler::compile_observed(source.into(), workers, check).is_err(),
                    "accepted {source}"
                );
            }
        }
    }
}
