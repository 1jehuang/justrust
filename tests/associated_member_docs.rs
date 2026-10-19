//! Original noncallable associated declaration documentation attachment.
use justrust_engine::compiler;
use std::{fs, os::unix::fs::PermissionsExt, process::Command};
#[path = "support/oracle.rs"]
mod oracle;

// Unchanged declaration/documentation excerpt from time-core 0.1.2 convert.rs.
// Copyright (c) 2022 Jacob Pratt et al. Licensed MIT OR Apache-2.0 (see the unchanged
// registry fixture's LICENSE-MIT and LICENSE-APACHE).
const SOURCE: &str = r#"
mod sealed {
    /// A trait for defining the ratio of two units of time.
    ///
    /// This trait is used to implement the `per` method on the various structs.
    pub trait Per<T> {
        /// The smallest unsigned integer type that can represent [`VALUE`](Self::VALUE).
        type Output;

        /// The number of one unit of time in the other.
        const VALUE: Self::Output;
    }
}
struct Day;
struct Week;
impl sealed::Per<Week> for Day {
    #[doc = "The output type."]
    type Output = u8;
    #[doc(hidden)]
    const VALUE: u8 = 7;
}
impl Day {
    /// An inherent constant.
    const N: u8 = 3;
}
fn main() { println!("{}", <Day as sealed::Per<Week>>::VALUE + Day::N); }
"#;

#[test]
fn original_associated_member_docs_match_stock_check_build_run_workers1_8() {
    oracle::check(
        "associated-member-docs",
        &[oracle::Case {
            name: "unchanged_time_core_per_header_and_inherent_docs",
            source: SOURCE,
            expect: oracle::Expect::Run("10\n"),
        }],
    );
    for workers in [1, 8] {
        assert!(
            compiler::compile_observed(SOURCE.into(), workers, true)
                .unwrap()
                .executable
                .is_none()
        );
        let built = compiler::compile_observed(SOURCE.into(), workers, false).unwrap();
        let base = std::env::var_os("JCODE_SCRATCH_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!(
            "jr-associated-member-docs-{}-{workers}",
            std::process::id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let image = dir.join("native");
        fs::write(&image, built.executable.unwrap()).unwrap();
        fs::set_permissions(&image, fs::Permissions::from_mode(0o700)).unwrap();
        let run = Command::new(image).output().unwrap();
        assert!(run.status.success(), "{run:?}");
        assert_eq!(run.stdout, b"10\n");
        fs::remove_dir_all(dir).unwrap();
    }
}

#[test]
fn docs_do_not_hide_unused_associated_member_errors() {
    for source in [
        "trait T{/// documented requirement\n const N:u8;}impl T for u8{#[doc=\"bad\"]const N:bool=true;}fn main(){}",
        "trait T{/// documented default\n const N:u8=true;}fn main(){}",
        "trait T{/// documented output\n type A;}impl T for u8{#[doc=\"unresolved\"]type A=Missing;}fn main(){}",
    ] {
        for workers in [1, 8] {
            for check in [true, false] {
                let error = compiler::compile_observed(source.into(), workers, check)
                    .err()
                    .expect("unused invalid declaration must fail")
                    .to_string();
                assert!(
                    !error.to_lowercase().contains("unsupported"),
                    "{source}: {error}"
                );
            }
        }
    }
}
