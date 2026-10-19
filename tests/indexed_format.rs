//! Indexed formatting borrows the original place and evaluates indices once.
use justrust_engine::compiler;
use std::os::unix::fs::PermissionsExt;

#[test]
fn indexed_format_matches_stock() {
    let source = include_str!("fixtures/indexed_format/transport.rs");
    let expected = include_bytes!("fixtures/indexed_format/transport.stdout");
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.into(), workers, check)
                .unwrap_or_else(|error| panic!("workers={workers},check={check}: {error}"));
            assert_eq!(result.executable.is_none(), check);
            if let Some(bytes) = result.executable {
                let root = std::env::var_os("JCODE_SCRATCH_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir);
                let path = root.join(format!("indexed-format-{}-{workers}", std::process::id()));
                std::fs::write(&path, bytes).unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
                let output = std::process::Command::new(&path).output().unwrap();
                std::fs::remove_file(path).unwrap();
                assert!(output.status.success(), "{output:?}");
                assert_eq!(&output.stdout, expected);
            }
        }
    }
}

#[test]
fn indexed_format_keeps_borrow_live_across_later_arguments() {
    let source = include_str!("fixtures/indexed_format/conflict.rs");
    for workers in [1, 8] {
        for check in [true, false] {
            let error = match compiler::compile(source.into(), workers, check) {
                Ok(_) => panic!("format borrow overlap accepted"),
                Err(error) => error.to_string(),
            };
            assert!(
                !error.to_ascii_lowercase().contains("unsupported"),
                "{error}"
            );
        }
    }
}
