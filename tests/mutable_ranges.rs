//! Mutable range reborrows. Frozen contract: MUTABLE-RANGES.md.
use justrust_engine::compiler;
use std::os::unix::fs::PermissionsExt;

#[test]
fn mutable_range_transport_matches_stock() {
    let source = include_str!("fixtures/mutable_ranges/transport.rs");
    let expected = include_bytes!("fixtures/mutable_ranges/transport.stdout");
    for workers in [1, 8] {
        for check in [true, false] {
            let result = compiler::compile(source.into(), workers, check)
                .unwrap_or_else(|error| panic!("workers={workers},check={check}: {error}"));
            assert_eq!(result.executable.is_none(), check);
            if let Some(bytes) = result.executable {
                let root = std::env::var_os("JCODE_SCRATCH_DIR")
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(std::env::temp_dir);
                let path = root.join(format!("mutable-range-{}-{workers}", std::process::id()));
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
fn mutable_range_reborrows_check() {
    let source = "fn part<'a>(s: &'a mut [i32]) -> &'a mut [i32] { &mut s[1..] } fn main() { let mut a = [1, 2, 3]; let s = part(&mut a); s[0] = 9; let view = &s[..1]; let n = view.len(); println!(\"{}\", n); }";
    for workers in [1, 8] {
        let result = compiler::compile(source.into(), workers, true)
            .unwrap_or_else(|error| panic!("workers={workers}: {error}"));
        assert!(result.executable.is_none());
    }
}

#[test]
fn overlapping_mutable_ranges_reject() {
    let source =
        "fn main(){let mut a=[1,2,3]; let x=&mut a[..2]; let y=&mut a[1..]; x[0]=8; y[0]=9;}";
    for workers in [1, 8] {
        let result = compiler::compile(source.into(), workers, true);
        assert!(
            result.is_err(),
            "overlapping ranges accepted, workers={workers}"
        );
    }
}
