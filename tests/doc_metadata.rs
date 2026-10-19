//! Original-source rustdoc configuration, not execution of doctests or rustdoc.
use justrust_engine::compiler;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
#[path = "support/oracle.rs"]
mod oracle;

const ORIGINAL_METADATA: &str = r#"
#![doc(html_favicon_url = "https://avatars0.githubusercontent.com/u/55999857")]
#![doc(html_logo_url = "https://avatars0.githubusercontent.com/u/55999857")]
#![doc(test(attr(deny(warnings))))]
fn main(){println!("ok");}
"#;

#[test]
fn original_rustdoc_metadata_matches_stock_and_native_workers1_8() {
    oracle::check(
        "rustdoc-metadata",
        &[
            oracle::Case {
                name: "original_time_core_metadata",
                source: ORIGINAL_METADATA,
                expect: oracle::Expect::Run("ok\n"),
            },
            oracle::Case {
                name: "nested_doctest_cfg_is_not_native_cfg",
                source: "#![doc(test(attr(cfg(any()),allow(dead_code))))]\nfn main(){println!(\"ok\");}",
                expect: oracle::Expect::Run("ok\n"),
            },
        ],
    );
    for workers in [1, 8] {
        let checked = compiler::compile_observed(ORIGINAL_METADATA.into(), workers, true).unwrap();
        assert!(checked.executable.is_none());
        let built = compiler::compile_observed(ORIGINAL_METADATA.into(), workers, false).unwrap();
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let directory = base.join(format!("jr-doc-metadata-{}-{workers}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let image = directory.join("native");
        fs::write(&image, built.executable.unwrap()).unwrap();
        fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
        let run = Command::new(&image).output().unwrap();
        assert!(run.status.success(), "{run:?}");
        assert_eq!(run.stdout, b"ok\n");
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn doc_settings_never_hide_unused_native_errors_or_grant_attributes() {
    for workers in [1, 8] {
        for check_only in [true, false] {
            for source in [
                "#![doc(test(attr(cfg(any()))))] fn bad()->u8{true} fn main(){}",
                "#![doc(html_logo_url=\"x\")] #[unknown_native_attribute] fn main(){}",
                "#![doc(test(attr(deny(warnings) allow(dead_code))))] fn main(){}",
            ] {
                assert!(
                    compiler::compile_observed(source.into(), workers, check_only).is_err(),
                    "accepted {source}"
                );
            }
        }
    }
}
