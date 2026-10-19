//! Frozen range evaluation-order and trap-prefix controls.
use justrust_engine::compiler;
use std::os::unix::fs::PermissionsExt;

#[test]
fn mutable_range_order_and_traps_match_stock() {
    let fixtures = [
        (
            "order",
            include_str!("fixtures/mutable_ranges/order.rs"),
            include_bytes!("fixtures/mutable_ranges/order.stdout").as_slice(),
            false,
        ),
        (
            "reverse",
            include_str!("fixtures/mutable_ranges/reverse.rs"),
            b"before\n".as_slice(),
            true,
        ),
        (
            "beyond",
            include_str!("fixtures/mutable_ranges/beyond.rs"),
            b"before\n".as_slice(),
            true,
        ),
        (
            "overflow",
            include_str!("fixtures/mutable_ranges/overflow.rs"),
            b"before\n".as_slice(),
            true,
        ),
    ];
    for (name, source, expected, trap) in fixtures {
        for workers in [1, 8] {
            for check in [true, false] {
                let result = compiler::compile(source.into(), workers, check)
                    .unwrap_or_else(|e| panic!("{name}, workers={workers}, check={check}: {e}"));
                assert_eq!(result.executable.is_none(), check);
                if let Some(bytes) = result.executable {
                    let root = std::env::var_os("JCODE_SCRATCH_DIR")
                        .map(std::path::PathBuf::from)
                        .unwrap_or_else(std::env::temp_dir);
                    let path = root.join(format!("range-{name}-{}-{workers}", std::process::id()));
                    std::fs::write(&path, bytes).unwrap();
                    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                        .unwrap();
                    let output = std::process::Command::new(&path).output().unwrap();
                    std::fs::remove_file(path).unwrap();
                    assert_eq!(!output.status.success(), trap, "{name}: {output:?}");
                    assert_eq!(&output.stdout, expected, "{name}");
                }
            }
        }
    }
}
